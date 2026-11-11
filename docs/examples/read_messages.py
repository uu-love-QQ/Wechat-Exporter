#!/usr/bin/env python3
"""读取解密后的微信消息，并处理常见的 zstd、XML、引用和 appmsg 类型。

用法：
    python read_messages.py /path/to/decrypted/db_storage 123456789@chatroom
    python read_messages.py /path/to/decrypted/db_storage wxid_xxx --limit 20
    python read_messages.py /path/to/decrypted/db_storage wxid_xxx --json

依赖：
    Python 3.10+
    可选：pip install zstandard
    如果系统有 zstd 命令，脚本会在没有 zstandard 时自动调用 zstd -d。
"""
from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import shutil
import subprocess
import sqlite3
import sys
from pathlib import Path

try:
    import zstandard  # type: ignore
except ImportError:
    zstandard = None

ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"
SENDER_PREFIX = re.compile(r"^([A-Za-z0-9_@\-.]+):\r?\n", re.S)


def quote_ident(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def open_readonly(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def zstd_decompress(data: bytes) -> bytes:
    if zstandard is not None:
        return zstandard.ZstdDecompressor().decompress(data)
    executable = shutil.which("zstd")
    if not executable:
        raise RuntimeError("需要 zstd 解压：pip install zstandard，或安装 zstd 命令")
    result = subprocess.run([executable, "-d", "-q", "-c"], input=data, stdout=subprocess.PIPE)
    if result.returncode != 0:
        raise RuntimeError("zstd 解压失败")
    return result.stdout


def decode_content(value, content_type) -> str:
    """把 TEXT/BLOB 内容和 WCDB_CT_* 转成文本。"""
    if value is None:
        return ""
    if isinstance(value, str):
        data = value.encode("utf-8", "surrogateescape")
    elif isinstance(value, bytes):
        data = value
    else:
        data = str(value).encode("utf-8")

    if content_type == 4 or data.startswith(ZSTD_MAGIC):
        data = zstd_decompress(data)

    return data.decode("utf-8", "replace").rstrip("\x00")


def normalize_xml(text: str) -> str:
    text = text.strip().lstrip("\ufeff")
    start = text.find("<")
    end = text.rfind(">")
    return text[start:end + 1] if start >= 0 and end > start else text


def clean_text(value: str) -> str:
    value = html.unescape(value)
    value = re.sub(r"<[^>]+>", " ", value)
    return re.sub(r"[ \t]+", " ", value).strip()


def xml_value(xml: str, tag: str) -> str:
    match = re.search(rf"<{re.escape(tag)}\b[^>]*>(.*?)</{re.escape(tag)}>", xml, re.S | re.I)
    if not match:
        return ""
    value = match.group(1).replace("<![CDATA[", "").replace("]]>", "")
    return clean_text(value)


def app_type(xml: str, fallback: int) -> int:
    text = xml_value(xml, "type")
    try:
        return int(text)
    except ValueError:
        return fallback


def parse_quote(xml: str) -> dict[str, str]:
    match = re.search(r"<refermsg>(.*?)</refermsg>", xml, re.S | re.I)
    if not match:
        return {}
    block = match.group(1)
    return {
        "sender": xml_value(block, "displayname"),
        "content": xml_value(block, "content"),
        "type": xml_value(block, "type"),
    }


def readable_app_text(xml: str, subtype: int, fallback: str) -> str:
    title = xml_value(xml, "title")
    if subtype == 57:
        quote = parse_quote(xml)
        reply = title or "引用消息"
        if quote:
            return f"{reply}\n引用 {quote.get('sender', '')}：{quote.get('content', '')}"
        return reply
    if subtype in (5, 49):
        url = xml_value(xml, "url")
        return f"{title} {url}".strip() if title or url else "链接消息"
    if subtype == 6:
        return f"[文件] {title}".strip()
    if subtype in (51, 63, 115):
        return f"[视频号] {title}".strip()
    if subtype == 19:
        return f"[聊天记录] {title}".strip()
    if subtype in (33, 36):
        return f"[小程序] {title}".strip()
    if subtype == 87:
        return f"[群公告] {xml_value(xml, 'textannouncement') or title}".strip()
    return title or fallback


def type_info(local_type: int, content: str) -> tuple[int, int, str]:
    base = local_type & 0xFFFFFFFF
    subtype = local_type >> 32
    xml = normalize_xml(content) if content.lstrip().startswith("<") else ""
    if base == 49:
        subtype = app_type(xml, subtype)
    names = {
        1: "文本", 3: "图片", 34: "语音", 37: "好友请求", 42: "名片",
        43: "视频", 47: "动画表情", 48: "位置", 49: "应用消息",
        50: "通话", 10000: "系统消息", 10002: "系统消息",
    }
    app_names = {
        3: "音乐", 5: "链接/公众号文章", 6: "文件", 19: "聊天记录",
        33: "小程序", 36: "小程序", 51: "视频号", 57: "引用",
        62: "拍一拍", 63: "视频号", 87: "群公告", 115: "视频号",
        2000: "转账",
    }
    if base == 49:
        name = app_names.get(subtype, "应用消息")
    else:
        name = names.get(base, f"其他({base})")
    return base, subtype, name


def load_name2id(conn: sqlite3.Connection) -> dict[int, str]:
    try:
        return {rowid: user for rowid, user in conn.execute(
            "SELECT rowid, user_name FROM Name2Id"
        ) if user}
    except sqlite3.Error:
        return {}


def find_table(conn: sqlite3.Connection, conversation_id: str) -> str | None:
    wanted = "Msg_" + hashlib.md5(conversation_id.encode("utf-8")).hexdigest()
    names = [row[0] for row in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'Msg_%'"
    )]
    for name in names:
        if name.lower() == wanted.lower():
            return name
    return None


def read_messages(db_storage: Path, conversation_id: str, limit: int) -> list[dict]:
    rows: list[dict] = []
    for path in sorted((db_storage / "message").glob("*.db")):
        if path.name.endswith("_fts.db") or path.name == "message_resource.db":
            continue
        try:
            conn = open_readonly(path)
        except sqlite3.Error:
            continue
        try:
            table = find_table(conn, conversation_id)
            if not table:
                continue
            names = load_name2id(conn)
            sql = (
                f"SELECT local_id,server_id,local_type,sort_seq,real_sender_id,create_time,"
                f"source,message_content,compress_content,WCDB_CT_message_content,WCDB_CT_source "
                f"FROM {quote_ident(table)} ORDER BY create_time, sort_seq, local_id LIMIT ?"
            )
            for (local_id, server_id, local_type, sort_seq, sender_rowid, create_time,
                 source, content, compress, content_ct, source_ct) in conn.execute(sql, (limit,)):
                text = decode_content(content, content_ct)
                if not text and compress:
                    text = decode_content(compress, content_ct)
                source_text = decode_content(source, source_ct)
                prefix = SENDER_PREFIX.match(text)
                if prefix:
                    text = text[prefix.end():]
                base, subtype, type_name = type_info(local_type, text)
                if base == 49:
                    xml = normalize_xml(text)
                    text = readable_app_text(xml, subtype, text)
                rows.append({
                    "database": path.name,
                    "table": table,
                    "local_id": local_id,
                    "server_id": str(server_id),
                    "local_type": local_type,
                    "sort_seq": sort_seq,
                    "sender_id": names.get(sender_rowid, str(sender_rowid)),
                    "create_time": create_time,
                    "type": type_name,
                    "base_type": base,
                    "subtype": subtype,
                    "text": text,
                    "source": source_text,
                })
                if len(rows) >= limit:
                    return rows
        finally:
            conn.close()
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description="读取解密后的微信消息")
    parser.add_argument("db_storage", type=Path, help="解密后的 db_storage 目录")
    parser.add_argument("conversation_id", help="会话 ID，例如 wxid_xxx 或 xxx@chatroom")
    parser.add_argument("--limit", type=int, default=50, help="最多读取多少条，默认 50")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    args = parser.parse_args()

    if not (args.db_storage / "message").is_dir():
        print(f"找不到 {args.db_storage / 'message'}，请先解密 db_storage")
        return 2

    messages = read_messages(args.db_storage, args.conversation_id, args.limit)
    if not messages:
        print("没有找到消息。请确认会话 ID 和 message/message_*.db 是否正确。")
        return 1

    if args.json:
        print(json.dumps(messages, ensure_ascii=False, indent=2))
        return 0

    for message in messages:
        print(
            f"[{message['create_time']}] {message['sender_id']} "
            f"{message['type']} local_id={message['local_id']} "
            f"server_id={message['server_id']}"
        )
        print(f"  {message['text']}")
        if message["source"]:
            print(f"  source: {message['source'][:180]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
