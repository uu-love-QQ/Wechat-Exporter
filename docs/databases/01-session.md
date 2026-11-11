# session/session.db：会话、未读、草稿

## 1. 这个库是做什么的

`session/session.db` 保存**会话列表**，不是聊天内容本身。

可以把它理解成微信聊天列表：

```text
谁 → 最后一条消息是什么 → 有多少未读 → 什么时候排序 → 有没有草稿
```

真正的聊天消息在：

```text
message/message_0.db
message/message_1.db
...
```

所以想显示“会话列表 + 每个会话的消息”，通常需要：

1. 读 `session.db` 得到会话；
2. 用会话 `username` 计算 `Msg_<md5(username)>`；
3. 去 `message/message_N.db` 找消息表。

## 2. 实际表清单

本次 `out/` 解密后共有 7 张表：

| 表 | 本次行数 | 作用 |
|---|---:|---|
| `SessionTable` | 917 | 会话主表 |
| `Name2Id` | 855 | 未读表用的用户 ID 映射 |
| `SessionUnreadListTable_1` | 5723 | 每个会话的未读消息 |
| `SessionUnreadStatTable_1` | 27263 | 未读统计 |
| `SessionDeleteTable` | 32 | 已删除会话 |
| `SessionNoContactInfoTable` | 13 | 无联系人信息时的会话标题 |
| `SessionDraft` | 0 | 草稿 |

> 行数只是本次样本，不是固定值。

---

## 3. SessionTable

### 3.1 实际建表语句

```sql
CREATE TABLE SessionTable(
    username TEXT PRIMARY KEY,
    type INTEGER,
    unread_count INTEGER,
    unread_first_msg_srv_id INTEGER,
    unread_first_pat_msg_local_id INTEGER,
    unread_first_pat_msg_sort_seq INTEGER,
    is_hidden INTEGER,
    summary TEXT,
    draft TEXT,
    status INTEGER,
    last_timestamp INTEGER,
    sort_timestamp INTEGER,
    last_clear_unread_timestamp INTEGER,
    last_msg_locald_id INTEGER,
    last_msg_type INTEGER,
    last_msg_sub_type INTEGER,
    last_msg_sender TEXT,
    last_sender_display_name TEXT,
    last_msg_ext_type INTEGER
);
```

索引：

```sql
CREATE INDEX SessionTable_LSENDER
ON SessionTable(last_msg_sender);

CREATE INDEX SessionTable_TYPE
ON SessionTable(type);
```

### 3.2 字段逐个解释

| 字段 | 类型 | 含义 | 小白说明 |
|---|---|---|---|
| `username` | TEXT | 会话 ID | 私聊是 `wxid_xxx`，群聊是 `xxx@chatroom`，公众号是 `gh_xxx` |
| `type` | INTEGER | 会话类型 | **本次样本全部为 0**，不要用它判断群聊 |
| `unread_count` | INTEGER | 未读数 | 聊天列表右上角数字 |
| `unread_first_msg_srv_id` | INTEGER | 未读起点 server_id | 对应消息表 `server_id` |
| `unread_first_pat_msg_local_id` | INTEGER | 未读拍一拍 local_id | 拍一拍专用 |
| `unread_first_pat_msg_sort_seq` | INTEGER | 未读拍一拍排序 | 拍一拍专用 |
| `is_hidden` | INTEGER | 是否隐藏会话 | 1 表示隐藏 |
| `summary` | TEXT | 最后消息摘要 | 列表显示的文字 |
| `draft` | TEXT | 草稿 | 有时为空 |
| `status` | INTEGER | 状态位 | 版本差异较大 |
| `last_timestamp` | INTEGER | 最后消息时间 | Unix 秒 |
| `sort_timestamp` | INTEGER | 会话排序时间 | 本次样本大量为 0 |
| `last_clear_unread_timestamp` | INTEGER | 清空未读时间 | Unix 秒 |
| `last_msg_locald_id` | INTEGER | 最后消息 local_id | 注意字段名少了 `_i_`，拼写就是 `locald` |
| `last_msg_type` | INTEGER | 最后消息基础类型 | 不是完整 `local_type` |
| `last_msg_sub_type` | INTEGER | 最后消息子类型 | appmsg 子类型等 |
| `last_msg_sender` | TEXT | 最后消息发送者 | wxid 或群成员 |
| `last_sender_display_name` | TEXT | 最后发送者显示名 | 列表显示名 |
| `last_msg_ext_type` | INTEGER | 扩展类型 | 版本差异较大 |

### 3.3 小白最常用查询

按最后消息时间倒序：

```python
import sqlite3
from datetime import datetime
from pathlib import Path

DB = Path(r"C:\Users\你\Downloads\db_decrypted\session\session.db")
conn = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)

sql = """
SELECT username, unread_count, last_timestamp, summary
FROM SessionTable
ORDER BY last_timestamp DESC
LIMIT 20
"""

for username, unread, ts, summary in conn.execute(sql):
    time_text = datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M") if ts else ""
    print(f"{time_text} | 未读 {unread} | {username} | {summary}")

conn.close()
```

### 3.4 为什么 `type` 不可靠

本次数据：

```sql
SELECT type, count(*) FROM SessionTable GROUP BY type;
```

结果：

```text
0|917
```

所有会话的 `type` 都是 0。实际判断群聊更可靠的方法是：

```python
is_group = username.endswith("@chatroom")
```

或者用 `contact.db` 的 `local_type`：

```text
local_type = 2  -> 群聊
```

---

## 4. Name2Id

```sql
CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY
);
```

这个表用于给未读列表中的 `username_id` 做映射。

### 4.1 为什么需要它

`SessionUnreadListTable_1` 里存的是：

```text
username_id = 3
```

而不是直接存 `wxid_xxx`。

需要查：

```sql
SELECT rowid, user_name FROM Name2Id WHERE rowid = 3;
```

### 4.2 Python 示例

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\session\session.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

id_to_name = {
    rowid: name
    for rowid, name in conn.execute("SELECT rowid, user_name FROM Name2Id")
}

for username_id, unread in conn.execute(
    "SELECT username_id, unread_stat FROM SessionUnreadStatTable_1 LIMIT 10"
):
    print(id_to_name.get(username_id, username_id), unread)

conn.close()
```

---

## 5. 未读相关表

### 5.1 SessionUnreadListTable_1

```sql
CREATE TABLE SessionUnreadListTable_1(
    username_id INTEGER,
    server_id INTEGER,
    create_time INTEGER,
    local_id INTEGER,
    CONSTRAINT _NameId_ServerId PRIMARY KEY(username_id, server_id)
);
```

含义：

| 字段 | 作用 |
|---|---|
| `username_id` | 本库 `Name2Id.rowid` |
| `server_id` | 对应消息的 server_id |
| `create_time` | 消息时间，Unix 秒 |
| `local_id` | 对应消息 local_id |

一个会话可能有多行，表示未读消息列表。

### 5.2 SessionUnreadStatTable_1

```sql
CREATE TABLE SessionUnreadStatTable_1(
    username_id INTEGER,
    unread_stat INTEGER,
    CONSTRAINT _NameId PRIMARY KEY(username_id)
);
```

`unread_stat` 是该会话的未读统计值。

### 5.3 实际查询：找未读会话

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\session\session.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

sql = """
SELECT n.user_name, s.unread_stat
FROM SessionUnreadStatTable_1 s
JOIN Name2Id n ON n.rowid = s.username_id
WHERE s.unread_stat > 0
ORDER BY s.unread_stat DESC
LIMIT 20
"""

for username, unread in conn.execute(sql):
    print(username, unread)

conn.close()
```

---

## 6. 其他表

### 6.1 SessionDeleteTable

```sql
CREATE TABLE SessionDeleteTable(
    username TEXT PRIMARY KEY,
    delete_time INTEGER
);
```

记录删除的会话。`delete_time` 是 Unix 秒。

注意：这里删除的是**会话记录**，不代表消息表 `Msg_*` 一定被删除。

### 6.2 SessionNoContactInfoTable

```sql
CREATE TABLE SessionNoContactInfoTable(
    username TEXT PRIMARY KEY,
    session_title TEXT
);
```

有些会话没有联系人资料，微信会在这里保存一个临时标题。

### 6.3 SessionDraft

```sql
CREATE TABLE SessionDraft(
    username TEXT,
    window_id TEXT,
    timestamp INTEGER,
    draft_data BLOB
);

CREATE UNIQUE INDEX SessionDraft_SESSION_DRAFT
ON SessionDraft(username, window_id);
```

字段：

| 字段 | 作用 |
|---|---|
| `username` | 会话 ID |
| `window_id` | 窗口标识 |
| `timestamp` | 草稿时间 |
| `draft_data` | 草稿 BLOB，不能直接当文本 |

本次样本 `SessionDraft` 为 0 行。

---

## 7. 会话和消息如何关联

### 7.1 会话 ID -> 消息表

```python
import hashlib

conversation_id = "123456789@chatroom"
table = "Msg_" + hashlib.md5(conversation_id.encode("utf-8")).hexdigest()
print(table)
```

### 7.2 在消息库中找这张表

```python
import sqlite3
from pathlib import Path

root = Path(r"C:\Users\你\Downloads\db_decrypted")
conversation_id = "123456789@chatroom"
wanted = "Msg_" + hashlib.md5(conversation_id.encode()).hexdigest()

for db_path in sorted((root / "message").glob("message_*.db")):
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    tables = {
        row[0].lower()
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'Msg_%'"
        )
    }
    if wanted.lower() in tables:
        print("找到：", db_path)
        break
    conn.close()
```

### 7.3 读取最后一条消息

```python
sql = """
SELECT local_id, server_id, local_type, create_time,
       real_sender_id, message_content, WCDB_CT_message_content
FROM "Msg_xxx"
ORDER BY create_time DESC, sort_seq DESC, local_id DESC
LIMIT 1
"""
```

---

## 8. 常见问题

### 8.1 会话列表有，但消息为空

可能原因：

- 消息表在另一个 `message_N.db`；
- 消息表名 MD5 不对；
- 消息已经被清理；
- 你打开的是旧 WAL 快照。

### 8.2 `last_timestamp` 是 0

- 会话可能从未有消息；
- 或该字段在历史版本中不使用；
- 可以回退 `sort_timestamp` 或消息表内的最大 `create_time`。

### 8.3 会话未读数对不上

- `unread_count` 和 `SessionUnreadStatTable_1` 可能不同步；
- 版本差异下，以实际 `SessionTable.unread_count` 为准更直观；
- 不要修改数据库来“修未读”，只读分析即可。

### 8.4 会话在 `SessionDeleteTable` 里

说明会话被删除过。若消息表仍存在，消息可能还能读出来；是否显示取决于微信自己的状态。

---

## 9. 一页速查

```text
session/session.db
├── SessionTable                  会话主表
├── Name2Id                       未读表 ID 映射
├── SessionUnreadListTable_1      未读消息列表
├── SessionUnreadStatTable_1      未读统计
├── SessionDeleteTable            已删除会话
├── SessionNoContactInfoTable     无联系人标题
└── SessionDraft                  草稿
```

最常用字段：

```text
SessionTable.username
SessionTable.unread_count
SessionTable.last_timestamp
SessionTable.summary
SessionTable.last_msg_sender
```

最常用查询：

```sql
SELECT username, unread_count, last_timestamp, summary
FROM SessionTable
ORDER BY last_timestamp DESC;
```
