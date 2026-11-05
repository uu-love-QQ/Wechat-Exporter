use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Layout {
    pub account_root: Option<PathBuf>,
    pub session_db: Option<PathBuf>,
    pub contact_db: Option<PathBuf>,
    pub message_dbs: Vec<PathBuf>,
    /// `db_storage` root, used to locate the media database for voice payloads.
    pub db_storage: Option<PathBuf>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct CandidateDatabase {
    pub db_storage: PathBuf,
    pub session_db: PathBuf,
    pub account_root: PathBuf,
    pub account_id: String,
    pub modified_time: SystemTime,
}

pub fn is_message_shard(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    if !name.ends_with(".db") {
        return false;
    }
    let stem = name.trim_end_matches(".db");
    let Some(rest) = stem.strip_prefix("message_") else {
        if let Some(rest) = stem.strip_prefix("biz_message_") {
            return rest.chars().all(|c| c.is_ascii_digit());
        }
        return false;
    };
    rest.chars().all(|c| c.is_ascii_digit())
}

fn message_shard_rank(path: &Path) -> (u8, u64) {
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let (prefix, rest) = if let Some(rest) = name.strip_prefix("message_") {
        (0u8, rest.trim_end_matches(".db"))
    } else if let Some(rest) = name.strip_prefix("biz_message_") {
        (1u8, rest.trim_end_matches(".db"))
    } else {
        (2u8, "")
    };
    (prefix, rest.parse::<u64>().unwrap_or(u64::MAX))
}

fn find_named_child(root: &Path, target: &str, max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, target: &str, depth: usize, max_depth: usize) -> Option<PathBuf> {
        if depth > max_depth {
            return None;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return None;
        };
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if entry.file_name().to_string_lossy() == target {
                    return Some(path);
                }
            } else if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if matches!(name.as_str(), "cache" | "temp" | "tmp" | "backup" | "db_storage") {
                    continue;
                }
                dirs.push(path);
            }
        }
        for dir in dirs {
            if let Some(found) = walk(&dir, target, depth + 1, max_depth) {
                return Some(found);
            }
        }
        None
    }
    walk(root, target, 0, max_depth)
}

fn find_db_storage(start: &Path) -> Option<PathBuf> {
    if start.join("db_storage").is_dir() {
        return Some(start.join("db_storage"));
    }
    if start.file_name().and_then(|v| v.to_str()) == Some("db_storage") {
        return Some(start.to_path_buf());
    }
    if start.join("session").join("session.db").is_file() {
        return Some(start.to_path_buf());
    }
    if start.join("message").is_dir() {
        return Some(start.to_path_buf());
    }
    // Bounded search for a db_storage directory below the supplied path.
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return None;
        };
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "db_storage" {
                return Some(path);
            }
            if matches!(
                name.as_str(),
                "cache" | "temp" | "tmp" | "backup" | "msg" | "FileStorage" | "node_modules"
            ) {
                continue;
            }
            children.push(path);
        }
        for child in children {
            if let Some(found) = walk(&child, depth + 1) {
                return Some(found);
            }
        }
        None
    }
    walk(start, 0)
}

fn query_registry_wechat_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let output = Command::new("reg")
            .args(["query", "HKCU\\Software\\Tencent\\WeChat", "/v", "FileSavePath"])
            .output()
            .ok()?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains("FileSavePath") {
                    if let Some(idx) = line.find("REG_SZ") {
                        let path_str = line[idx + 6..].trim();
                        if !path_str.eq_ignore_ascii_case("MyDocument:") && !path_str.is_empty() {
                            let p = PathBuf::from(path_str);
                            if p.exists() {
                                return Some(p);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn candidate_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    // 1. Environment variable overrides
    for env_var in [
        "ALPHA_WECHAT_ROOT",
        "XWECHAT_FILES_DIR",
        "WECHAT_FILES_DIR",
        "WECHAT_DATA_DIR",
    ] {
        if let Ok(val) = std::env::var(env_var) {
            let p = PathBuf::from(val);
            if p.exists() {
                roots.push(p.clone());
                roots.push(p.join("xwechat_files"));
            }
        }
    }

    // 2. Windows registry query (if WeChat FileSavePath was customized)
    if let Some(reg_path) = query_registry_wechat_path() {
        roots.push(reg_path.clone());
        roots.push(reg_path.join("xwechat_files"));
        roots.push(reg_path.join("WeChat Files").join("xwechat_files"));
    }

    // 3. Windows standard user profile paths
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile);
        roots.push(p.join("Documents").join("xwechat_files"));
        roots.push(p.join("文档").join("xwechat_files"));
        roots.push(p.join("xwechat_files"));
        roots.push(p.join("AppData").join("Roaming").join("Tencent").join("WeChat").join("xwechat_files"));
        roots.push(p.join("AppData").join("Local").join("Tencent").join("WeChat").join("xwechat_files"));
    }

    // 4. Windows drive root and depth-1 subdirectories (detects migrations like 电脑管家迁移文件)
    let drive_letters = [
        "C", "D", "E", "F", "G", "H", "I", "J",
        "c", "d", "e", "f", "g", "h", "i", "j",
    ];
    for letter in drive_letters {
        let drive_root_str = format!("{letter}:\\");
        let drive_path = Path::new(&drive_root_str);
        if !drive_path.exists() {
            continue;
        }

        // Direct paths on drive root
        for sub_name in [
            "xwechat_files",
            "WeChat Files\\xwechat_files",
            "微信文件\\xwechat_files",
            "电脑管家迁移文件\\xwechat_files",
            "Documents\\xwechat_files",
            "文档\\xwechat_files",
        ] {
            let sub = drive_path.join(sub_name);
            if sub.exists() {
                roots.push(sub);
            }
        }

        // Traverse level-1 subdirectories on drive (finds custom migrated folders)
        if let Ok(entries) = fs::read_dir(drive_path) {
            for entry in entries.flatten() {
                let sub_dir = entry.path();
                if !sub_dir.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('$') || matches!(
                    name.as_str(),
                    "Windows" | "Program Files" | "Program Files (x86)" | "System Volume Information" | "Recovery" | "PerfLogs"
                ) {
                    continue;
                }

                let c1 = sub_dir.join("xwechat_files");
                if c1.is_dir() {
                    roots.push(c1);
                }
                let c2 = sub_dir.join("WeChat Files").join("xwechat_files");
                if c2.is_dir() {
                    roots.push(c2);
                }
            }
        }
    }

    // 5. Linux standard paths
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home);
        roots.push(p.join("xwechat_files"));
        roots.push(p.join("Documents").join("xwechat_files"));
        roots.push(p.join("文档").join("xwechat_files"));
        roots.push(p.join("Downloads").join("xwechat_files"));
        roots.push(p.join("Desktop").join("xwechat_files"));
        roots.push(p.join("桌面").join("xwechat_files"));

        // Flatpak / native WeChat on Linux
        roots.push(p.join(".var/app/com.tencent.WeChat/data/xwechat_files"));
        roots.push(p.join(".local/share/com.tencent.wechat/xwechat_files"));
        roots.push(p.join(".config/wechat/xwechat_files"));

        // Wine / Deepin-Wine
        let wine_docs = p.join(".wine/drive_c/users");
        if wine_docs.is_dir() {
            if let Ok(entries) = fs::read_dir(&wine_docs) {
                for entry in entries.flatten() {
                    roots.push(entry.path().join("Documents").join("xwechat_files"));
                    roots.push(entry.path().join("xwechat_files"));
                }
            }
        }
        let deepin_docs = p.join(".deepinwine/Deepin-WeChat/drive_c/users");
        if deepin_docs.is_dir() {
            if let Ok(entries) = fs::read_dir(&deepin_docs) {
                for entry in entries.flatten() {
                    roots.push(entry.path().join("Documents").join("xwechat_files"));
                    roots.push(entry.path().join("xwechat_files"));
                }
            }
        }
    }

    // 6. Temporary fixture path for tests
    let fixture_root = PathBuf::from("/tmp/wechat-fixture/xwechat_files");
    if fixture_root.exists() {
        roots.push(fixture_root);
    }

    roots.retain(|p| p.exists());
    roots.sort();
    roots.dedup();
    roots
}

pub fn auto_detect_candidates() -> Vec<CandidateDatabase> {
    let roots = candidate_search_roots();
    let mut candidates = Vec::new();

    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || matches!(name.as_str(), "cache" | "temp" | "tmp" | "backup" | "All Users")
            {
                continue;
            }

            let db_storage = if path.join("db_storage").is_dir() {
                path.join("db_storage")
            } else if path.file_name().and_then(|v| v.to_str()) == Some("db_storage") {
                path.clone()
            } else if path.join("session").join("session.db").is_file() {
                path.clone()
            } else {
                continue;
            };

            let session_db = db_storage.join("session").join("session.db");
            if !session_db.is_file() {
                continue;
            }

            // Check if there is at least one message shard
            let message_dir = db_storage.join("message");
            let has_shard = if message_dir.is_dir() {
                fs::read_dir(&message_dir)
                    .map(|entries| entries.flatten().any(|e| is_message_shard(&e.path())))
                    .unwrap_or(false)
            } else {
                false
            };

            if !has_shard {
                continue;
            }

            let mtime = session_db
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);

            let account_root = if db_storage.file_name().and_then(|v| v.to_str()) == Some("db_storage") {
                db_storage.parent().unwrap_or(&path).to_path_buf()
            } else {
                path.clone()
            };

            let account_id = account_root
                .file_name()
                .and_then(|v| v.to_str())
                .map(crate::util::clean_account_id)
                .unwrap_or_else(|| "unknown".to_string());

            candidates.push(CandidateDatabase {
                db_storage,
                session_db,
                account_root,
                account_id,
                modified_time: mtime,
            });
        }
    }

    candidates.sort_by(|a, b| b.modified_time.cmp(&a.modified_time));
    candidates.dedup_by(|a, b| a.db_storage == b.db_storage);
    candidates
}

pub fn format_scanned_roots_message() -> String {
    let roots = candidate_search_roots();
    let mut msg = String::from("未在默认路径中找到有效的微信 4.x 数据库。\n检索了以下位置：\n");
    if roots.is_empty() {
        msg.push_str("  (系统中未检测到微信默认存储文件夹)\n");
    } else {
        for root in roots {
            msg.push_str(&format!("  - {}\n", root.display()));
        }
    }
    msg.push_str("\n请使用 --source-database <PATH> 手动指定 session.db 或 db_storage 目录，\n或设置环境变量 XWECHAT_FILES_DIR=/path/to/xwechat_files。");
    msg
}

pub fn discover(source: &Path) -> Result<Layout> {
    let source = super::super::util::normalize_path(source);
    let (db_storage, mut message_dbs) = if source.is_dir() {
        let ds = find_db_storage(&source)
            .with_context(|| format!("could not find session/session.db below {}", source.display()))?;
        (ds, Vec::new())
    } else {
        let name = source.file_name().and_then(|v| v.to_str()).unwrap_or("");
        if !name.ends_with(".db") {
            bail!("source database must be a .db file, db_storage directory, or account directory");
        }
        let mut ds = None;
        if is_message_shard(&source) || name == "session.db" || name == "contact.db" {
            if let Some(parent) = source.parent() {
                if name == "session.db" && parent.file_name().and_then(|v| v.to_str()) == Some("session") {
                    ds = parent.parent().map(|p| p.to_path_buf());
                } else if parent.file_name().and_then(|v| v.to_str()) == Some("db_storage") {
                    ds = Some(parent.to_path_buf());
                } else {
                    ds = parent.parent().map(|p| p.to_path_buf());
                }
            }
        }
        let ds = ds.unwrap_or_else(|| source.parent().unwrap_or(Path::new(".")).to_path_buf());
        let shards = if is_message_shard(&source) {
            vec![source.clone()]
        } else {
            Vec::new()
        };
        (ds, shards)
    };

    let account_root = if db_storage.file_name().and_then(|v| v.to_str()) == Some("db_storage") {
        db_storage.parent().map(|p| p.to_path_buf())
    } else {
        None
    };

    let message_dir = db_storage.join("message");
    if message_dbs.is_empty() {
        if message_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&message_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && is_message_shard(&path) {
                        message_dbs.push(path);
                    }
                }
            }
        }
    }
    message_dbs.sort_by_key(|path| message_shard_rank(path));
    message_dbs.dedup();

    let session_db = {
        let direct = db_storage.join("session").join("session.db");
        if direct.is_file() {
            Some(direct)
        } else {
            find_named_child(&db_storage, "session.db", 3)
        }
    };
    let contact_db = {
        let direct = db_storage.join("contact").join("contact.db");
        if direct.is_file() {
            Some(direct)
        } else {
            find_named_child(&db_storage, "contact.db", 3)
        }
    };

    if message_dbs.is_empty() {
        bail!(
            "no message shards (message_N.db) found below {}",
            db_storage.display()
        );
    }

    Ok(Layout {
        account_root,
        session_db,
        contact_db,
        message_dbs,
        db_storage: Some(db_storage),
    })
}
