# 消息库内部表与工具表

## 1. 这个文档讲什么

消息分片里除了 `Msg_*`，还有一些内部表：

- `TimeStamp`
- `DeleteInfo`
- `DeleteResInfo`
- `SendInfo`
- `HistorySysMsgInfo`
- `HistoryAddMsgInfo`
- `MessageGroupTimeInfo`
- `wcdb_builtin_compression_record`
- `alpha_message_recall_v1`
- `alpha_message_recall_state_v1`

它们不是聊天正文，但会影响：

- 消息是否被删除；
- 历史消息索引；
- 发送状态；
- 时间线；
- 压缩记录；
- 某些工具的恢复状态。

## 2. 实际表清单（message_0.db）

```text
DeleteInfo
DeleteResInfo
HistoryAddMsgInfo
HistorySysMsgInfo
SendInfo
TimeStamp
alpha_message_recall_state_v1
alpha_message_recall_v1
wcdb_builtin_compression_record
sqlite_sequence
```

不同分片可能没有 `SendInfo`、`MessageGroupTimeInfo`。

## 3. TimeStamp

```sql
CREATE TABLE TimeStamp(
    timestamp INTEGER
);
```

通常只有一行，表示数据库快照/更新时间，Unix 秒。

### 3.1 Python

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\message\message_0.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
print(conn.execute("SELECT timestamp FROM TimeStamp").fetchall())
```

## 4. DeleteInfo

```sql
CREATE TABLE DeleteInfo(
    chat_name_id INTEGER,
    delete_table_name TEXT,
    CONSTRAINT UNIQUE_CHAT_DELETE UNIQUE(chat_name_id, delete_table_name)
);
```

记录某会话删除过哪张消息表。

| 字段 | 作用 |
|---|---|
| `chat_name_id` | 会话内部 ID |
| `delete_table_name` | 被删除的表名 |

注意：它记录“删除事件/表名”，不等于 `Msg_*` 表一定不存在。

## 5. DeleteResInfo

```sql
CREATE TABLE DeleteResInfo(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_name_id INTEGER,
    msg_create_time INTEGER,
    msg_local_id INTEGER,
    res_path TEXT
);
```

删除的媒体资源：

| 字段 | 作用 |
|---|---|
| `session_name_id` | 会话内部 ID |
| `msg_create_time` | 消息时间 |
| `msg_local_id` | 消息 local_id |
| `res_path` | 资源路径 |

### 5.1 Python

```python
for row in conn.execute("""
    SELECT session_name_id, msg_create_time, msg_local_id, res_path
    FROM DeleteResInfo
    LIMIT 20
"""):
    print(row)
```

## 6. SendInfo

```sql
CREATE TABLE SendInfo(
    chat_name_id INTEGER,
    msg_local_id INTEGER
);
```

发送状态辅助表。不同版本可能没有。

## 7. HistorySysMsgInfo / HistoryAddMsgInfo

```sql
CREATE TABLE HistorySysMsgInfo(
    session_name_id INTEGER,
    history_id INTEGER,
    server_id INTEGER,
    is_revoke INTEGER,
    CONSTRAINT _UNIQUEID PRIMARY KEY(session_name_id, history_id, server_id)
);

CREATE TABLE HistoryAddMsgInfo(
    session_name_id INTEGER,
    history_id INTEGER,
    server_id INTEGER,
    is_revoke INTEGER,
    CONSTRAINT _UNIQUEID PRIMARY KEY(session_name_id, history_id, server_id)
);
```

| 字段 | 作用 |
|---|---|
| `session_name_id` | 会话内部 ID |
| `history_id` | 历史 ID |
| `server_id` | 消息 server_id |
| `is_revoke` | 是否撤回 |

用于历史消息/系统消息的索引，不是消息正文。

## 8. MessageGroupTimeInfo

只在部分分片出现：

```sql
CREATE TABLE MessageGroupTimeInfo(
    chatname_id INTEGER,
    group_id TEXT,
    create_time INTEGER,
    initial_sort_seq INTEGER,
    birth_time INTEGER
);
```

| 字段 | 作用 |
|---|---|
| `chatname_id` | 会话内部 ID |
| `group_id` | 群/会话哈希 |
| `create_time` | 群创建/时间线时间 |
| `initial_sort_seq` | 初始排序，13 位 |
| `birth_time` | 出生/创建时间，秒 |

### 8.1 Python

```python
for row in conn.execute("""
    SELECT chatname_id, group_id, create_time, initial_sort_seq, birth_time
    FROM MessageGroupTimeInfo
    LIMIT 20
"""):
    print(row)
```

## 9. wcdb_builtin_compression_record

```sql
CREATE TABLE wcdb_builtin_compression_record(
    tableName TEXT PRIMARY KEY,
    columns TEXT NOT NULL,
    rowid INTEGER
) WITHOUT ROWID;
```

WCDB 内部压缩元数据：

| 字段 | 作用 |
|---|---|
| `tableName` | 表名 |
| `columns` | 压缩列 |
| `rowid` | 记录 ID |

当前样本为空，但它是 WCDB 自己的表，不要删除。

## 10. alpha_message_recall_v1

实际 `message_0..3.db` 中出现：

```sql
CREATE TABLE alpha_message_recall_v1(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    source_table TEXT NOT NULL,
    local_id INTEGER NOT NULL,
    server_id TEXT NOT NULL,
    source_type INTEGER NOT NULL,
    sent_at INTEGER NOT NULL,
    sender TEXT NOT NULL,
    message_content BLOB,
    compress_content BLOB,
    packed_info_data BLOB,
    source BLOB,
    wcdb_content_type INTEGER,
    wcdb_source_type INTEGER,
    captured_at INTEGER NOT NULL,
    confirmation TEXT NOT NULL,
    confirmed_at INTEGER NOT NULL DEFAULT 0,
    byte_size INTEGER NOT NULL
);
```

这看起来像某个 Alpha/恢复工具为了保存“被撤回/被删除消息”创建的表。

字段：

| 字段 | 作用 |
|---|---|
| `conversation_id` | 会话 ID |
| `source_table` | 来源 `Msg_*` 表 |
| `local_id` | 原消息 local_id |
| `server_id` | 原消息 server_id |
| `source_type` | 原消息 local_type |
| `sent_at` | 消息时间 |
| `sender` | 发送者 |
| `message_content` | 原始内容 BLOB |
| `compress_content` | 压缩内容 |
| `packed_info_data` | 媒体信息 |
| `source` | 来源 XML |
| `wcdb_content_type` | CT |
| `wcdb_source_type` | source CT |
| `captured_at` | 捕获时间 |
| `confirmation` | 确认状态 |
| `confirmed_at` | 确认时间 |
| `byte_size` | 字节数 |

## 11. alpha_message_recall_state_v1

```sql
CREATE TABLE alpha_message_recall_state_v1(
    conversation_id TEXT PRIMARY KEY,
    dropped_rows INTEGER NOT NULL DEFAULT 0
);
```

工具状态表，记录某会话丢弃/处理了多少行。当前样本为空。

## 12. 重要结论

- `alpha_*` **不是微信 WCDB 原生表**；
- 备份/恢复时不要把 `alpha_*` 当消息主表；
- `DeleteInfo`/`DeleteResInfo` 是删除辅助，不是恢复数据本身；
- `History*` 是索引，不是消息正文；
- `wcdb_builtin_compression_record` 是 WCDB 内部表；
- 恢复消息时优先使用 `Msg_*` + `Name2Id` + `SessionTable` + FTS。

## 13. 速查

```text
message/message_N.db
├── Msg_*                         消息正文
├── Name2Id                       发送者
├── TimeStamp                     快照时间
├── DeleteInfo                    删除表记录
├── DeleteResInfo                 删除资源
├── SendInfo                      发送状态
├── HistorySysMsgInfo             历史系统消息
├── HistoryAddMsgInfo             历史新增消息
├── MessageGroupTimeInfo          群时间线
├── wcdb_builtin_compression_record  WCDB 内部压缩
├── alpha_message_recall_v1       工具表（非原生）
└── alpha_message_recall_state_v1 工具表（非原生）
```

关键点：

1. 内部表不要当聊天正文；
2. `alpha_*` 不是微信原生；
3. 恢复时不要删除 `wcdb_builtin_compression_record`；
4. `TimeStamp` 可帮助判断快照时间；
5. 删除记录不代表消息一定还能恢复。
