#!/usr/bin/env python3
"""列出解密后 db_storage 中每个 SQLite 数据库的表、字段和行数。

用法：
    python inspect_wcdb.py /path/to/decrypted/db_storage
    python inspect_wcdb.py /path/to/decrypted/db_storage --schema message/message_0.db

这个脚本只读取普通 SQLite 文件，不负责 SQLCipher 解密。
如果是微信原始加密文件，请先用 wechat-exporter decrypt 解密。
"""
from __future__ import annotations

import argparse
import sqlite3
from pathlib import Path


def quote_ident(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def open_readonly(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def iter_databases(root: Path):
    if root.is_file():
        yield root
        return
    for path in sorted(root.rglob("*.db")):
        yield path


def print_table(conn: sqlite3.Connection, table: str) -> None:
    try:
        count = conn.execute(f"SELECT count(*) FROM {quote_ident(table)}").fetchone()[0]
    except sqlite3.Error as error:
        count = f"不可直接读取：{error}"
    columns = [row[1] for row in conn.execute(f"PRAGMA table_info({quote_ident(table)})")]
    print(f"    {table}: rows={count}, columns={', '.join(columns)}")


def main() -> int:
    parser = argparse.ArgumentParser(description="列出解密后的微信 WCDB 表结构")
    parser.add_argument("root", type=Path, help="解密后的 db_storage 目录或单个 .db 文件")
    parser.add_argument("--schema", help="只显示指定的相对数据库路径，例如 message/message_0.db")
    parser.add_argument("--max-tables", type=int, default=30, help="每个库最多显示多少张表")
    args = parser.parse_args()

    if not args.root.exists():
        print(f"路径不存在：{args.root}")
        return 2

    for path in iter_databases(args.root):
        relative = path.relative_to(args.root) if args.root.is_dir() else path.name
        if args.schema and str(relative).replace("\\", "/") != args.schema.replace("\\", "/"):
            continue
        print(f"\n=== {relative} ===")
        try:
            conn = open_readonly(path)
        except sqlite3.Error as error:
            print(f"  打开失败：{error}")
            continue
        try:
            tables = [row[0] for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
            )]
            print(f"  表数量：{len(tables)}")
            for table in tables[: args.max_tables]:
                print_table(conn, table)
            if len(tables) > args.max_tables:
                print(f"  ... 还有 {len(tables) - args.max_tables} 张表未显示")
        except sqlite3.DatabaseError as error:
            print(f"  不是可读取的普通 SQLite：{error}")
            print("  如果这是加密库，请先运行：wechat-exporter decrypt ...")
        finally:
            conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
