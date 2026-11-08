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

#[test]
fn lists_conversations() {
    let (_temp, source) = fixture();
    let (code, stdout, stderr) = run_with_key(&[
        "list",
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
    assert!(!conversations.iter().any(|item| item["id"] == "wxid_testuser"));
    assert_eq!(value["account_id"], "wxid_testuser");
}

#[test]
fn exports_all_formats_and_media() {
    let (temp, source) = fixture();
    let output = temp.path().join("out");
    let (code, stdout, stderr) = run_with_key(&[
        "export",
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
        "conversation.weclone.csv",
    ] {
        assert!(output.join(name).exists(), "missing {name}");
    }
    assert!(output.join("export-report.json").exists());
    let json: Value =
        serde_json::from_slice(&fs::read(output.join("conversation.json")).unwrap()).unwrap();
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 11);
    assert!(messages.iter().any(|message| message["content"] == "hello group"));
    assert!(messages
        .iter()
        .any(|message| message["content"] == "compressed hello"));
    assert!(messages
        .iter()
        .any(|message| message["content"]
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
    assert!(markdown.contains("文件: report.html (未索引，空缺)"), "{markdown}");
    let html = fs::read_to_string(output.join("conversation.html")).unwrap();
    assert!(!html.contains("__CHAT_DATA__"), "template placeholder left behind");
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
    assert!(messages.iter().all(|message| message.get("source").is_none()));
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("export-report.json")).unwrap()).unwrap();
    assert_eq!(report["conversations"][0]["message_count"], 11);
    assert!(report["conversations"][0]["media"]["saved"].as_u64().unwrap() >= 1);
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
fn help_scopes_options_to_subcommands() {
    let root = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .arg("--help")
        .output()
        .expect("root help");
    let root = String::from_utf8_lossy(&root.stdout);
    for command in ["init", "detect", "list", "search", "export"] {
        assert!(root.contains(command), "missing subcommand {command}");
    }
    assert!(!root.contains("--conversation-id"));

    let list = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .args(["list", "--help"])
        .output()
        .expect("list help");
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(list.contains("--page-size"));
    assert!(!list.contains("--conversation-id"));
    assert!(!list.contains("--output-dir"));
    assert!(!list.contains("--format"));

    let export = Command::new(env!("CARGO_BIN_EXE_wechat-exporter"))
        .args(["export", "--help"])
        .output()
        .expect("export help");
    let export = String::from_utf8_lossy(&export.stdout);
    assert!(export.contains("--conversation-id"));
    assert!(export.contains("--format"));
    assert!(!export.contains("--page-size"));
    assert!(!export.contains("--ascending"));
}

#[test]
fn exported_chatlab_matches_spec_shape() {
    let (temp, source) = fixture();
    let output = temp.path().join("chatlab");
    let (code, _, stderr) = run_with_key(&[
        "export",
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
        serde_json::from_slice(&fs::read(output.join("conversation.chatlab.json")).unwrap()).unwrap();
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
        .args(["list", "--key", KEY, "--page-size", "10"]);
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
