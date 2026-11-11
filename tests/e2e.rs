use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const GROUP: &str = "123456789@chatroom";
const ALICE: &str = "wxid_alice";

fn fixture() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let root = temp.path().join("data");
    let status = Command::new(env!("CARGO_BIN_EXE_wechat-fixture"))
        .arg(&root)
        .status()
        .expect("run fixture");
    assert!(status.success());
    let source = root
        .join("xwechat_files")
        .join("wxid_testuser_abcd")
        .join("db_storage");
    (temp, source)
}

fn run_with_key(args: &[&str]) -> (i32, String, String) {
    let mut output = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .args(args)
        .args(["--key", KEY])
        .output()
        .expect("run alpha-cli");
    if output.status.success() {
        output.stdout.extend_from_slice(&[]);
    }
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn embedded_report(html: &str) -> Value {
    assert!(
        !html.contains("__REPORT_DATA__"),
        "report template placeholder left behind"
    );
    let marker = "id=\"report-data\" type=\"application/json\">";
    let start = html.find(marker).expect("report data island") + marker.len();
    let end = html[start..].find("</script>").expect("report data end") + start;
    serde_json::from_str(&html[start..end]).expect("embedded report json")
}

#[test]
fn lists_conversations() {
    let (_temp, source) = fixture();
    let (code, stdout, stderr) = run_with_key(&[
        "conversation",
        "--source-database",
        source.to_str().unwrap(),
        "--page-size",
        "10",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json");
    let conversations = value["conversations"].as_array().expect("array");
    assert!(conversations.iter().any(|item| item["id"] == GROUP));
    assert!(conversations.iter().any(|item| item["id"] == ALICE));
    assert!(!conversations
        .iter()
        .any(|item| item["id"] == "wxid_testuser"));
    assert_eq!(value["account_id"], "wxid_testuser");
}

#[test]
fn exports_all_formats_and_media() {
    let (temp, source) = fixture();
    let output = temp.path().join("out");
    let (code, stdout, stderr) = run_with_key(&[
        "conversation",
        "--export",
        "--source-database",
        source.to_str().unwrap(),
        "--conversation-id",
        GROUP,
        "--format",
        "all",
        "--output-dir",
        output.to_str().unwrap(),
        "--include-media",
    ]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    for name in [
        "conversation.json",
        "conversation.md",
        "conversation.chatlab.json",
        "conversation.chatlab.jsonl",
        "conversation.html",
        "conversation.txt",
        "conversation.xlsx",
        "conversation.arkme.json",
        "conversation.sql",
        "conversation.sqlite",
        "conversation.weclone.csv",
    ] {
        assert!(output.join(name).exists(), "missing {name}");
    }
    assert!(output.join("export-report.json").exists());
    let json: Value =
        serde_json::from_slice(&fs::read(output.join("conversation.json")).unwrap()).unwrap();
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 11);
    assert!(messages
        .iter()
        .any(|message| message["content"] == "hello group"));
    assert!(messages
        .iter()
        .any(|message| message["content"] == "compressed hello"));
    assert!(messages.iter().any(|message| message["content"]
        .as_str()
        .unwrap()
        .contains("original text")));
    assert!(messages.iter().any(|message| message["fromMe"] == true));
    assert!(messages.iter().any(|message| message["fromMe"] == false));
    assert!(messages.iter().any(|message| message["media"]["path"]
        .as_str()
        .unwrap_or("")
        .starts_with("media/image/")));
    let voice = messages
        .iter()
        .find(|message| message["type"] == "语音消息")
        .expect("voice message");
    let voice_path = voice["media"]["path"].as_str().expect("voice media path");
    assert!(voice_path.starts_with("media/voice/"), "{voice_path}");
    assert!(voice_path.ends_with(".wav"), "{voice_path}");
    let wav = fs::read(output.join(voice_path)).expect("voice wav");
    assert!(wav.starts_with(b"RIFF"));
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24000);
    let missing = messages
        .iter()
        .find(|message| message["media"]["name"] == "report.html")
        .expect("file message");
    assert!(missing["media"].get("path").is_none());
    assert_eq!(missing["media"]["status"], "missing");
    let markdown = fs::read_to_string(output.join("conversation.md")).unwrap();
    assert!(
        markdown.contains("文件: report.html (未索引，空缺)"),
        "{markdown}"
    );
    let html = fs::read_to_string(output.join("conversation.html")).unwrap();
    assert!(
        !html.contains("__CHAT_DATA__"),
        "template placeholder left behind"
    );
    let marker = "id=\"chat-data\" type=\"application/json\">";
    let start = html.find(marker).expect("chat data island") + marker.len();
    let end = html[start..].find("</script>").expect("data end") + start;
    let data: Value = serde_json::from_str(&html[start..end]).expect("embedded chat json");
    assert_eq!(data["messages"].as_array().unwrap().len(), 11);
    let embedded = data["messages"].as_array().unwrap();
    assert!(embedded
        .iter()
        .any(|message| message["missing"]["name"] == "report.html"));
    let record = embedded
        .iter()
        .find(|message| message["record"]["title"] == "草帽Wantu与Harry的聊天记录")
        .expect("chat record");
    assert_eq!(record["record"]["items"].as_array().unwrap().len(), 2);
    let linked = embedded
        .iter()
        .find(|message| message["quote"]["sender"] == "Alice")
        .expect("linked quote");
    assert!(linked["quote"]["target"].is_number(), "linked quote target");
    let lost = embedded
        .iter()
        .find(|message| message["quote"]["sender"] == "Ghost")
        .expect("missing quote");
    assert!(lost["quote"]["target"].is_null(), "missing quote target");
    assert!(messages
        .iter()
        .all(|message| message.get("source").is_none()));
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("export-report.json")).unwrap()).unwrap();
    assert_eq!(report["conversations"][0]["message_count"], 11);
    assert!(report["conversations"][0]["start_time"].as_i64().unwrap() > 0);
    assert!(
        report["conversations"][0]["end_time"].as_i64().unwrap()
            >= report["conversations"][0]["start_time"].as_i64().unwrap()
    );
    assert!(report["start_time"].as_i64().unwrap() > 0);
    assert!(report["end_time"].as_i64().unwrap() >= report["start_time"].as_i64().unwrap());
    assert!(
        report["conversations"][0]["media"]["saved"]
            .as_u64()
            .unwrap()
            >= 1
    );
    let sqlite = rusqlite::Connection::open(output.join("conversation.sqlite")).unwrap();
    assert_eq!(
        sqlite
            .query_row("SELECT count(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        11
    );
    let stored: String = sqlite
        .query_row(
            "SELECT content FROM messages WHERE content = 'hello group'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, "hello group");
}

#[test]
fn recovers_from_sqlite_backup() {
    let (temp, source) = fixture();
    let backup = temp.path().join("sqlite-backup");
    let (code, _, stderr) = run_with_key(&[
        "conversation",
        "--export",
        "--source-database",
        source.to_str().unwrap(),
        "--conversation-id",
        GROUP,
        "--format",
        "sqlite",
        "--output-dir",
        backup.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let recovered = temp.path().join("recovered");
    let output = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .args([
            "recovery",
            "--input",
            backup.to_str().unwrap(),
            "--output-dir",
            recovered.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run recovery");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(recovered.join("recovery-report.json")).unwrap()).unwrap();
    assert_eq!(report["recovered"].as_array().unwrap().len(), 1);
    let folder = report["recovered"][0]["output"].as_str().unwrap();
    let json: Value =
        serde_json::from_slice(&fs::read(PathBuf::from(folder).join("conversation.json")).unwrap())
            .unwrap();
    assert_eq!(json["session"]["id"], GROUP);
    assert_eq!(json["messages"].as_array().unwrap().len(), 11);
}

#[test]
fn decrypts_databases_to_plaintext() {
    let (temp, source) = fixture();
    let output = temp.path().join("decrypted");
    let (code, stdout, stderr) = run_with_key(&[
        "decrypt",
        "--source-database",
        source.to_str().unwrap(),
        "--output-dir",
        output.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(value["failed"].as_array().unwrap().len(), 0);
    assert!(output.join("session/session.db").exists());
    let connection = rusqlite::Connection::open(output.join("session/session.db")).unwrap();
    let tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tables > 0);
    let header = fs::read(output.join("message/message_0.db")).unwrap();
    assert!(header.starts_with(b"SQLite format 3\0"));
}

#[test]
fn searches_across_shards() {
    let (_temp, source) = fixture();
    let (code, stdout, stderr) = run_with_key(&[
        "search",
        "hello",
        "--source-database",
        source.to_str().unwrap(),
        "--ascending",
        "--page-size",
        "10",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json");
    let items = value["items"].as_array().unwrap();
    assert!(items.iter().any(|item| item["conversation_id"] == ALICE));
    assert!(items.iter().any(|item| item["conversation_id"] == GROUP));
}

#[test]
fn list_query_filters_conversations() {
    let (_temp, source) = fixture();
    let (code, stdout, stderr) = run_with_key(&[
        "conversation",
        "-q",
        "Alice",
        "--source-database",
        source.to_str().unwrap(),
        "--page-size",
        "10",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(value["query"], "Alice");
    assert_eq!(value["total"], 1);
    let conversations = value["conversations"].as_array().unwrap();
    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0]["id"], ALICE);
    assert!(value["next_cursor"].is_null());
}

#[test]
fn exports_visual_report() {
    let (temp, source) = fixture();
    let output = temp.path().join("report-out");
    let (code, _, stderr) = run_with_key(&[
        "conversation",
        "--export",
        "--source-database",
        source.to_str().unwrap(),
        "--conversation-id",
        GROUP,
        "--format",
        "json",
        "--report",
        "--output-dir",
        output.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let report = output.join("conversation.report.html");
    assert!(report.exists(), "missing report html");
    let html = fs::read_to_string(&report).unwrap();
    let data = embedded_report(&html);
    assert_eq!(data["kind"], "group");
    assert!(data["kpis"].as_array().unwrap().len() >= 4);
    assert_eq!(data["members"].as_array().unwrap().len(), 3);
    assert!(!data["types"].as_array().unwrap().is_empty());
    assert!(!data["trend"].as_array().unwrap().is_empty());
    assert!(data["words"]["me"].as_array().unwrap().len() > 0);
    let exported: Value =
        serde_json::from_slice(&fs::read(output.join("export-report.json")).unwrap()).unwrap();
    assert_eq!(
        exported["conversations"][0]["documents"]["report"],
        "conversation.report.html"
    );
}

#[test]
fn configures_and_clears_webdav_backup() {
    let temp = tempfile::TempDir::new().unwrap();
    let binary = env!("CARGO_BIN_EXE_wechat-exporter");
    let output = Command::new(binary)
        .current_dir(temp.path())
        .args([
            "backup",
            "webdav",
            "--url",
            "http://127.0.0.1:9/dav",
            "--username",
            "user",
            "--password",
            "secret",
            "--remote-path",
            "backups",
        ])
        .output()
        .expect("configure backup");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temp.path().join(".wechat-exporter-backup.json").exists());

    let output = Command::new(binary)
        .current_dir(temp.path())
        .args(["backup", "show"])
        .output()
        .expect("show backup");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("http://127.0.0.1:9/dav"));
    assert!(stdout.contains("backups"));
    assert!(!stdout.contains("secret"));

    let output = Command::new(binary)
        .current_dir(temp.path())
        .args(["backup", "clear"])
        .output()
        .expect("clear backup");
    assert!(output.status.success());
    assert!(!temp.path().join(".wechat-exporter-backup.json").exists());
}

#[test]
fn generates_four_report_types() {
    let (temp, source) = fixture();
    let cases = [
        (
            "private",
            Vec::<&str>::new(),
            "private-report.html",
            "friends",
        ),
        (
            "group",
            vec!["--conversation-id", GROUP],
            "group-report.html",
            "members",
        ),
        (
            "annual",
            vec!["--year", "2023"],
            "annual-report.html",
            "groups",
        ),
        (
            "dual",
            vec!["--conversation-id", ALICE],
            "dual-report.html",
            "ratio",
        ),
    ];
    for (kind, extra, file, needle) in cases {
        let dir = temp.path().join(format!("report-{kind}"));
        let mut args = vec![
            "report",
            kind,
            "--source-database",
            source.to_str().unwrap(),
            "--output-dir",
            dir.to_str().unwrap(),
        ];
        args.extend(extra);
        let (code, stdout, stderr) = run_with_key(&args);
        assert_eq!(code, 0, "{kind} stderr={stderr} stdout={stdout}");
        let html = fs::read_to_string(dir.join(file)).unwrap();
        let data = embedded_report(&html);
        assert_eq!(data["kind"], kind, "{kind} kind mismatch");
        if needle == "ratio" {
            assert!(data[needle].is_object(), "{kind} missing {needle}");
        } else {
            assert!(data[needle].is_array(), "{kind} missing {needle}");
        }
    }
}

#[test]
fn reports_from_exported_json() {
    let (temp, source) = fixture();
    let exported = temp.path().join("json-src");
    let (code, _, stderr) = run_with_key(&[
        "conversation",
        "--export",
        "--source-database",
        source.to_str().unwrap(),
        "--format",
        "json",
        "--output-dir",
        exported.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let reports = temp.path().join("json-reports");
    fs::create_dir_all(&reports).unwrap();
    let cases = [
        ("private", None, "private-report.html", "friends"),
        ("group", Some(GROUP), "group-report.html", "members"),
        ("annual", None, "annual-report.html", "groups"),
        ("dual", Some(ALICE), "dual-report.html", "ratio"),
    ];
    for (kind, conversation, file, needle) in cases {
        let dir = reports.join(kind);
        let mut args = vec![
            "report",
            kind,
            "--from-json",
            exported.to_str().unwrap(),
            "--output-dir",
            dir.to_str().unwrap(),
        ];
        if let Some(id) = conversation {
            args.extend(["--conversation-id", id]);
        }
        let (code, stdout, stderr) = run_with_key(&args);
        assert_eq!(code, 0, "{kind} stderr={stderr} stdout={stdout}");
        let html = fs::read_to_string(dir.join(file)).unwrap();
        let data = embedded_report(&html);
        assert_eq!(data["kind"], kind, "{kind} kind mismatch");
        if needle == "ratio" {
            assert!(data[needle].is_object(), "{kind} missing {needle}");
        } else {
            assert!(data[needle].is_array(), "{kind} missing {needle}");
        }
    }
}

#[test]
fn help_scopes_options_to_subcommands() {
    let root = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .arg("--help")
        .output()
        .expect("root help");
    let root = String::from_utf8_lossy(&root.stdout);
    for command in ["init", "detect", "search", "conversation", "favorite"] {
        assert!(root.contains(command), "missing subcommand {command}");
    }
    assert!(!root.contains("--conversation-id"));

    let export = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .args(["conversation", "--help"])
        .output()
        .expect("conversation help");
    let export = String::from_utf8_lossy(&export.stdout);
    assert!(export.contains("--conversation-id"));
    assert!(export.contains("--format"));
    assert!(export.contains("--report"));
    assert!(export.contains("--page-size"));
    assert!(export.contains("--export"));
    assert!(!export.contains("--ascending"));
}

#[test]
fn exported_chatlab_matches_spec_shape() {
    let (temp, source) = fixture();
    let output = temp.path().join("chatlab");
    let (code, _, stderr) = run_with_key(&[
        "conversation",
        "--export",
        "--source-database",
        source.to_str().unwrap(),
        "--conversation-id",
        GROUP,
        "--format",
        "chatlab-json",
        "--format",
        "chatlab-jsonl",
        "--output-dir",
        output.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value =
        serde_json::from_slice(&fs::read(output.join("conversation.chatlab.json")).unwrap())
            .unwrap();
    assert_eq!(value["chatlab"]["version"], "0.0.2");
    assert_eq!(value["meta"]["platform"], "wechat");
    assert_eq!(value["meta"]["type"], "group");
    assert!(!value["members"].as_array().unwrap().is_empty());
    assert_eq!(value["messages"].as_array().unwrap().len(), 11);
    let lines = fs::read_to_string(output.join("conversation.chatlab.jsonl")).unwrap();
    let lines: Vec<&str> = lines.lines().collect();
    assert!(lines[0].contains("\"_type\":\"header\""));
    assert!(lines.last().unwrap().contains("\"_type\":\"message\""));
}

#[test]
fn auto_detects_database_and_lists() {
    let (_temp, source) = fixture();
    // Set XWECHAT_FILES_DIR to point to parent of account
    let parent = source.parent().unwrap().parent().unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_wechat-exporter"));
    cmd.env("XWECHAT_FILES_DIR", parent)
        .args(["conversation", "--key", KEY, "--page-size", "10"]);
    let output = cmd.output().expect("run alpha-cli auto-detect");
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("wxid_alice"));
    assert!(stdout.contains("123456789@chatroom"));
}

#[test]
fn detects_databases_subcommand() {
    let (_temp, source) = fixture();
    let parent = source.parent().unwrap().parent().unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_wechat-exporter"));
    cmd.env("XWECHAT_FILES_DIR", parent).arg("detect");
    let output = cmd.output().expect("run alpha-cli -D");
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("正在检索本机所有 WeChat 4.x 数据库"));
    assert!(stdout.contains("wxid_testuser"));
}

#[test]
fn restores_plaintext_backup_into_encrypted_target() {
    let (temp, source) = fixture();
    let backup = temp.path().join("decrypt-backup");
    let (code, _, stderr) = run_with_key(&[
        "decrypt",
        "--source-database",
        source.to_str().unwrap(),
        "--output-dir",
        backup.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let connection = rusqlite::Connection::open(backup.join("message/message_0.db")).unwrap();
    connection
        .execute(
            "UPDATE Msg_9234c5fa4065f721b47403f393822c44 SET message_content='RESTORED_MARKER' WHERE local_id=1",
            [],
        )
        .unwrap();
    drop(connection);

    let target_root = temp.path().join("target");
    fs::create_dir_all(&target_root).unwrap();
    let target = target_root.join("db_storage");
    copy_tree(&source, &target);
    let (code, stdout, stderr) = run_with_key(&[
        "recovery",
        "--input",
        backup.to_str().unwrap(),
        "--target-database",
        target.to_str().unwrap(),
        "--force",
    ]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["failed"].as_array().unwrap().len(), 0);

    let verified = temp.path().join("verified");
    let (code, _, stderr) = run_with_key(&[
        "decrypt",
        "--source-database",
        target.to_str().unwrap(),
        "--output-dir",
        verified.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let connection = rusqlite::Connection::open(verified.join("message/message_0.db")).unwrap();
    let content: String = connection
        .query_row(
            "SELECT message_content FROM Msg_9234c5fa4065f721b47403f393822c44 WHERE local_id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(content, "RESTORED_MARKER");
}

fn copy_tree(source: &std::path::Path, target: &std::path::Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = target.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}
