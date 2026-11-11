use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::cli::ValidatedCli;
use crate::db::Db;
use crate::wechat::discover::Layout;

fn collect_databases(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_databases(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn copy_sidecars(source: &Path, target: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{suffix}", source.display()));
        let to = PathBuf::from(format!("{}{suffix}", target.display()));
        if from.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn remove_database_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

fn decrypt_one(
    source: &Path,
    target: &Path,
    key: Option<&[u8; 32]>,
    plaintext: bool,
) -> Result<()> {
    if target.exists() {
        bail!("target database already exists: {}", target.display());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if plaintext {
        std::fs::copy(source, target)
            .with_context(|| format!("failed to copy {}", source.display()))?;
        copy_sidecars(source, target)?;
        Db::open(target, None, true)
            .context("decrypted copy is not a plaintext SQLite database")?;
        return Ok(());
    }
    let key = key.context("database key is required")?;
    let staging = PathBuf::from(format!("{}.staging", target.display()));
    let _ = std::fs::remove_file(&staging);
    std::fs::copy(source, &staging)
        .with_context(|| format!("failed to copy {}", source.display()))?;
    copy_sidecars(source, &staging)?;
    let db = Db::open_for_decrypt(&staging, Some(key), false)
        .with_context(|| format!("failed to open copied database {}", staging.display()))?;
    let export = db.decrypt_to(target);
    drop(db);
    remove_database_files(&staging);
    export?;
    Ok(())
}

pub fn run(options: &ValidatedCli, layout: &Layout) -> Result<()> {
    let source = layout
        .db_storage
        .as_deref()
        .context("source db_storage path is missing")?;
    let output = options
        .output_dir
        .as_deref()
        .context("decrypt output directory is missing")?;
    let mut databases = Vec::new();
    collect_databases(source, &mut databases)?;
    databases.sort();
    std::fs::create_dir_all(output)?;
    let mut decrypted = Vec::new();
    let mut failed = Vec::new();
    for path in databases {
        let relative = path.strip_prefix(source).unwrap_or(&path);
        let target = output.join(relative);
        let result = if target.exists() && !options.force {
            Err(anyhow::anyhow!("target exists"))
        } else {
            if target.exists() {
                remove_database_files(&target);
            }
            decrypt_one(&path, &target, options.key.as_ref(), options.plaintext)
        };
        match result {
            Ok(()) => decrypted.push(relative.to_string_lossy().replace('\\', "/")),
            Err(error) => {
                remove_database_files(&target);
                failed.push(format!("{}: {error:#}", relative.display()));
            }
        }
    }
    let value = serde_json::json!({
        "source": source,
        "output": output,
        "decrypted": decrypted,
        "failed": failed,
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    if !failed.is_empty() {
        bail!("{} database(s) could not be decrypted", failed.len());
    }
    Ok(())
}
