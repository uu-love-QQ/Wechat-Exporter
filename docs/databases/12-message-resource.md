# message/message_resource.db：消息资源与媒体详情

## 1. 这个库是做什么的

`message/message_resource.db` 是消息和媒体资源之间的索引：

- 哪条消息拥有哪个资源；
- 资源是图片、视频、文件还是其他；
- 资源大小、创建/访问时间、状态；
- 资源打包信息（`packed_info` BLOB）；
- 会话/发送者内部 ID；
- FTS 范围/删除信息。

它不是消息正文库，也不是图片/视频文件本身。

## 2. 实际表清单

本次样本 7 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `MessageResourceInfo` | 26349 | 消息资源主信息 |
| `MessageResourceDetail` | 111804 | 资源详情 |
| `ChatName2Id` | 368 | 会话内部 ID |
| `SenderName2Id` | 1892 | 发送者内部 ID |
| `FtsRange` | 1152 | 全文索引范围 |
| `FtsDeleteInfo` | 27 | FTS 删除范围 |
| `sqlite_sequence` | 2 | 自增 |

## 3. 实际 DDL

### 3.1 MessageResourceInfo

```sql
CREATE TABLE MessageResourceInfo(
    message_id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER,
    sender_id INTEGER,
    message_local_type INTEGER,
    message_create_time INTEGER,
    message_local_id INTEGER,
    message_svr_id INTEGER,
    message_origin_source INTEGER,
    packed_info BLOB
);
```

字段：

| 字段 | 作用 |
|---|---|
| `message_id` | 资源消息主 ID |
| `chat_id` | 会话内部 ID |
| `sender_id` | 发送者内部 ID |
| `message_local_type` | 消息 `local_type` |
| `message_create_time` | 消息时间，Unix 秒 |
| `message_local_id` | 消息 local_id |
| `message_svr_id` | 消息 server_id |
| `message_origin_source` | 消息来源 |
| `packed_info` | 资源打包 BLOB |

实际样本：

```text
message_id=6
chat_id=4
sender_id=6
message_local_type=25769803825   # 6<<32 | 49，文件消息
message_create_time=1753848347
message_local_id=26
message_svr_id=8977914141444740935
message_origin_source=10
packed_info=空 BLOB
```

### 3.2 MessageResourceDetail

```sql
CREATE TABLE MessageResourceDetail(
    resource_id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id INTEGER,
    type INTEGER,
    size INTEGER,
    create_time INTEGER,
    access_time INTEGER,
    status INTEGER,
    data_index TEXT,
    packed_info BLOB
);
```

| 字段 | 作用 |
|---|---|
| `resource_id` | 资源 ID |
| `message_id` | 关联 `MessageResourceInfo.message_id` |
| `type` | 资源类型 |
| `size` | 资源大小 |
| `create_time` | 创建时间 |
| `access_time` | 访问时间 |
| `status` | 状态 |
| `data_index` | 资源序号/索引 |
| `packed_info` | 资源打包信息 BLOB |

实际样本：

```text
resource_id=4
message_id=4
type=3735555
size=42
create_time=1751966721
access_time=0
status=0
data_index='0'
packed_info=BLOB
```

`type=3735555` 是一个组合类型值，不是简单文件扩展名；需要结合消息 XML/实现判断。

## 4. ChatName2Id / SenderName2Id

```sql
CREATE TABLE ChatName2Id(
    user_name TEXT PRIMARY KEY,
    update_time INTEGER
);

CREATE TABLE SenderName2Id(
    user_name TEXT PRIMARY KEY,
    update_time INTEGER
);
```

作用：

```text
MessageResourceInfo.chat_id
  -> ChatName2Id.rowid
  -> 会话 wxid

MessageResourceInfo.sender_id
  -> SenderName2Id.rowid
  -> 发送者 wxid
```

实际样本：

```text
ChatName2Id:
  20787163450@chatroom, update_time=0
  49943198224@chatroom, update_time=1766539393

SenderName2Id:
  wxid_xxx, update_time=1749914996
```

### 4.1 Python：还原会话和发送者

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\message\message_resource.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

chats = {rowid: name for rowid, name in conn.execute(
    "SELECT rowid, user_name FROM ChatName2Id"
)}
senders = {rowid: name for rowid, name in conn.execute(
    "SELECT rowid, user_name FROM SenderName2Id"
)}

sql = """
SELECT message_id, chat_id, sender_id, message_local_type,
       message_create_time, message_local_id, message_svr_id,
       message_origin_source
FROM MessageResourceInfo
ORDER BY message_id DESC
LIMIT 20
"""

for row in conn.execute(sql):
    message_id, chat_id, sender_id, local_type, ts, local_id, svr_id, origin = row
    print(
        message_id,
        chats.get(chat_id, chat_id),
        senders.get(sender_id, sender_id),
        local_type,
        ts,
        local_id,
        svr_id,
        origin,
    )
```

## 5. 资源详情

```python
sql = """
SELECT d.resource_id, d.message_id, d.type, d.size, d.data_index,
       i.message_local_id, i.message_svr_id
FROM MessageResourceDetail d
JOIN MessageResourceInfo i ON i.message_id = d.message_id
LIMIT 20
"""
for row in conn.execute(sql):
    print(row)
```

`packed_info` 是 BLOB，可能是 protobuf。不要直接当文本。

## 6. FtsRange / FtsDeleteInfo

```sql
CREATE TABLE FtsRange(
    session_id INTEGER,
    db_time_stamp INTEGER,
    start_local_id INTEGER,
    end_local_id INTEGER,
    range_type INTEGER,
    CONSTRAINT _CHAT_TIME_TYPE UNIQUE(session_id, db_time_stamp, range_type)
);

CREATE TABLE FtsDeleteInfo(
    session_id INTEGER,
    max_message_id INTEGER
);
```

用于消息全文索引的增量范围：

- `session_id`：会话内部 ID；
- `db_time_stamp`：快照时间；
- `start_local_id`/`end_local_id`：消息范围；
- `range_type`：范围类型；
- `FtsDeleteInfo`：某会话已删除到哪个消息 ID。

## 7. 实际关系图

```text
message/message_N.db                 message/message_resource.db
┌─────────────────────┐             ┌──────────────────────────┐
│ Msg_<hash>          │             │ ChatName2Id              │
│  local_id           │             │  rowid -> user_name      │
│  server_id          │             └───────────┬──────────────┘
│  local_type         │                         │ chat_id
│  create_time        │             ┌───────────▼──────────────┐
│  message_content    │             │ MessageResourceInfo      │
└─────────┬───────────┘             │  message_id              │
          │ local_id/server_id      │  message_local_id        │
          └────────────────────────►│  message_svr_id          │
                                      │  packed_info             │
                                      └───────────┬──────────────┘
                                                  │ message_id
                                      ┌───────────▼──────────────┐
                                      │ MessageResourceDetail    │
                                      │  resource_id             │
                                      │  type,size,status        │
                                      │  packed_info             │
                                      └──────────────────────────┘
```

## 8. 常见问题

### 8.1 `MessageResourceInfo` 和消息主表怎么关联

优先：

```text
message_local_id + message_create_time
```

或者：

```text
message_svr_id
```

不要只靠 `message_id`，它只是资源主 ID。

### 8.2 `chat_id` 不是 wxid

它是 `ChatName2Id.rowid`，必须 JOIN。

### 8.3 `type` 不是文件扩展名

`MessageResourceDetail.type` 是内部资源类型，可能是组合值。媒体类型优先看消息 `local_type` 和 XML。

### 8.4 `packed_info` 是乱码

它是二进制/protobuf，不要直接解码为字符串。

### 8.5 资源表有数据但文件不存在

资源表只是索引；真实图片/视频/文件可能在：

- `msg/attach`；
- `msg/video`；
- `FileStorage`；
- hardlink 索引对应路径。

详见 [08-hardlink.md](08-hardlink.md)。

## 9. 速查

```text
message/message_resource.db
├── MessageResourceInfo      消息资源主信息
├── MessageResourceDetail    资源大小/类型/状态
├── ChatName2Id              会话内部 ID
├── SenderName2Id            发送者内部 ID
├── FtsRange                 FTS 范围
└── FtsDeleteInfo            FTS 删除范围
```

关键点：

1. `chat_id`/`sender_id` 是内部 ID；
2. `message_id` 关联 Info 和 Detail；
3. 消息主表用 `local_id`/`server_id` 关联；
4. `packed_info` 是 BLOB；
5. 资源索引不等于文件本身。
