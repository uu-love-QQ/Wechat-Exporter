# message/message_N.db：聊天消息主库

## 1. 这个库是做什么的

`message/` 目录保存聊天记录，按分片命名：

```text
message/message_0.db
message/message_1.db
message/message_2.db
message/message_3.db

message/biz_message_0.db
message/biz_message_1.db
message/biz_message_2.db
```

- `message_N.db`：普通私聊/群聊；
- `biz_message_N.db`：公众号/服务号等业务消息；
- 两者都使用相同的 `Msg_<md5(会话ID)>` 消息表结构。

一个会话可能在不同分片中都有表，读取时要遍历所有分片。

## 2. 实际表清单（样本）

`message_0.db` 有 186 张表，其中 175 张是 `Msg_*`。不同分片表数量不同：

| 分片 | 表总数 | 消息表数量 |
|---|---:|---:|
| `message_0.db` | 186 | 175 |
| `message_1.db` | 190 | 179 |
| `message_2.db` | 130 | 119 |
| `message_3.db` | 116 | 107 |
| `biz_message_0.db` | 473 | 467 |
| `biz_message_1.db` | 10 | 4 |
| `biz_message_2.db` | 7 | 1 |

这些数字只是当前账号样本，不是固定值。

## 3. 表名规则

消息表名：

```text
Msg_<md5(会话 username)>
```

例如：

```python
import hashlib

conversation_id = "123456789@chatroom"
table = "Msg_" + hashlib.md5(conversation_id.encode("utf-8")).hexdigest()
print(table)
```

输出类似：

```text
Msg_9234c5fa4065f721b47403f393822c44
```

注意：

- MD5 输入必须是 UTF-8 原始字节；
- 表名通常在 SQLite 中大小写不敏感，但比较时建议忽略大小写；
- 群 ID、私聊 wxid、公众号 `gh_` 都按同一规则。

## 4. Msg_* 主表

### 4.1 实际 DDL

```sql
CREATE TABLE Msg_<md5(username)>(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id INTEGER,
    local_type INTEGER,
    sort_seq INTEGER,
    real_sender_id INTEGER,
    create_time INTEGER,
    status INTEGER,
    upload_status INTEGER,
    download_status INTEGER,
    server_seq INTEGER,
    origin_source INTEGER,
    source TEXT,
    message_content TEXT,
    compress_content TEXT,
    packed_info_data BLOB,
    WCDB_CT_message_content INTEGER DEFAULT NULL,
    WCDB_CT_source INTEGER DEFAULT NULL
);
```

### 4.2 字段总表

| 字段 | 实际类型 | 含义 | 特殊处理 |
|---|---|---|---|
| `local_id` | INTEGER | 会话内本地 ID | 自增；恢复去重 |
| `server_id` | INTEGER | 服务端 ID | 19 位常见，用 i64/u64 |
| `local_type` | INTEGER | 64 位打包类型 | 拆高低 32 位 |
| `sort_seq` | INTEGER | 排序/时间序列 | 实际 13 位，不要当小序号 |
| `real_sender_id` | INTEGER | 发送者映射 | 指向本库 `Name2Id.rowid` |
| `create_time` | INTEGER | 消息时间 | Unix 秒 |
| `status` | INTEGER | 状态 | 版本差异 |
| `upload_status` | INTEGER | 上传状态 | 版本差异 |
| `download_status` | INTEGER | 下载状态 | 版本差异 |
| `server_seq` | INTEGER | 服务端顺序 | 版本差异 |
| `origin_source` | INTEGER | 来源标志 | 版本差异 |
| `source` | TEXT | 来源 XML | 常为 zstd BLOB |
| `message_content` | TEXT | 主内容 | 可能 zstd BLOB |
| `compress_content` | TEXT | 备用压缩 | 主内容为空时回退 |
| `packed_info_data` | BLOB | 二进制媒体信息 | 常见 protobuf |
| `WCDB_CT_message_content` | INTEGER | 内容编码 | 4=zstd |
| `WCDB_CT_source` | INTEGER | source 编码 | 4=zstd |

### 4.3 实际样本

```text
local_id=3165
server_id=5753352451433616612
local_type=1
sort_seq=1781533534000
real_sender_id=1380
create_time=1781533534
source=BLOB(zstd)
message_content=TEXT
compress_content=TEXT(空)
packed_info_data=BLOB
WCDB_CT_message_content=0
WCDB_CT_source=4
```

解释：

- 消息是文本（base_type=1）；
- `source` 是 zstd；
- `message_content` 是直接文本；
- `sort_seq` 是 13 位，不是 1、2、3；
- 发送者要用本库 `Name2Id[1380]`。

### 4.4 为什么声明 TEXT 却是 BLOB

SQLite 是动态类型：

```text
declared TEXT  -> 可以存 TEXT
               -> 也可以存 BLOB
```

所以判断依据必须是 Python 读出来的类型：

```python
if isinstance(value, bytes):
    ...
elif isinstance(value, str):
    ...
```

## 5. 消息类型

### 5.1 拆包

```python
base_type = local_type & 0xFFFFFFFF
sub_type = local_type >> 32
```

### 5.2 基础类型

| base_type | 含义 |
|---:|---|
| 1 | 文本 |
| 3 | 图片 |
| 34 | 语音 |
| 37 | 好友请求 |
| 42 | 名片 |
| 43 | 视频 |
| 47 | 动画表情 |
| 48 | 位置 |
| 49 | 应用消息 appmsg |
| 50 | 通话 |
| 10000 / 10002 | 系统消息 |

### 5.3 appmsg 子类型

| sub_type | 含义 |
|---:|---|
| 3 | 音乐 |
| 5 | 链接/公众号文章 |
| 6 | 文件 |
| 8 | 视频/其它 |
| 19 | 聊天记录 |
| 33 / 36 | 小程序 |
| 51 / 63 / 115 | 视频号 |
| 57 | 引用 |
| 62 | 拍一拍 |
| 87 | 群公告 |
| 2000 | 转账 |
| 2001 等 | 扩展业务 |

实际 `local_type` 样本：

```text
1
3
43
47
48
10000
244813135921 = 57<<32 | 49   引用
266287972401 = 62<<32 | 49   拍一拍
21474836529  = 5<<32 | 49    链接/文章
25769803825  = 6<<32 | 49    文件
219043332145 = 51<<32 | 49   视频号
81604378673  = 19<<32 | 49   聊天记录
```

### 5.4 错误示例：不要用 `<patMsg>` 判断拍一拍

普通 appmsg 中常见：

```xml
<patMsg>
  <records><recordNum>0</recordNum></records>
</patMsg>
```

但它只是空元数据。正确判断：

```python
xml = normalized_xml
is_pat = (
    local_type == 266287972401
    or (local_type >> 32) == 62
    or "<pat>" in xml
    or '<sysmsg type="pat"' in xml
)
```

错误写法：

```python
# 会把引用、公众号文章、视频号误判为拍一拍
if "<pat" in xml:
    ...
```

## 6. Name2Id

### 6.1 实际 DDL

```sql
CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY,
    is_session INTEGER
);
```

不同版本可能是：

```sql
CREATE TABLE Name2Id(
    user_name TEXT,
    is_group INTEGER
);
```

### 6.2 实际样本

```text
rowid=1  -> wxid_ab2uavjoj2q612  is_session=0
rowid=8  -> wxid_xxx            is_session=0
rowid=6  -> 46005576326@chatroom is_session=1
```

不要跨分片复用 `rowid`：

```text
message_0.db 的 rowid=1 是 A
message_1.db 的 rowid=1 可能是 B
```

### 6.3 Python：读取发送者映射

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\message\message_0.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

name2id = {
    rowid: name
    for rowid, name in conn.execute(
        "SELECT rowid, user_name FROM Name2Id"
    )
    if name
}

print(name2id.get(8, "未知"))
```

## 7. 消息辅助表

### 7.1 TimeStamp

```sql
CREATE TABLE TimeStamp(
    timestamp INTEGER
);
```

通常是该数据库快照/更新时间，Unix 秒。

### 7.2 DeleteInfo

```sql
CREATE TABLE DeleteInfo(
    chat_name_id INTEGER,
    delete_table_name TEXT,
    CONSTRAINT UNIQUE_CHAT_DELETE UNIQUE(chat_name_id, delete_table_name)
);
```

记录某会话删除过哪张表，常用于清理/同步。

### 7.3 DeleteResInfo

```sql
CREATE TABLE DeleteResInfo(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_name_id INTEGER,
    msg_create_time INTEGER,
    msg_local_id INTEGER,
    res_path TEXT
);
```

删除的媒体资源信息：

| 字段 | 作用 |
|---|---|
| `session_name_id` | 会话内部 ID |
| `msg_create_time` | 消息时间 |
| `msg_local_id` | 消息 local_id |
| `res_path` | 资源路径 |

### 7.4 SendInfo

```sql
CREATE TABLE SendInfo(
    chat_name_id INTEGER,
    msg_local_id INTEGER
);
```

发送状态辅助表。

### 7.5 HistorySysMsgInfo / HistoryAddMsgInfo

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

历史系统消息/新增消息索引；`is_revoke` 表示是否撤回。

### 7.6 MessageGroupTimeInfo

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

实际样本：

```text
chatname_id=80
group_id=5b3036e96421aacde868217e43c45bdf
create_time=1785951368
initial_sort_seq=1785951368000
birth_time=1785951369
```

`initial_sort_seq` 是 13 位；`birth_time` 是秒。

## 8. 非微信原生表

实际 `message_0..3.db` 中有：

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

CREATE TABLE alpha_message_recall_state_v1(
    conversation_id TEXT PRIMARY KEY,
    dropped_rows INTEGER NOT NULL DEFAULT 0
);
```

这两张表不是微信 WCDB 原生 schema，疑似 Alpha/实验工具创建，当前样本为空。备份和恢复时不要把它们当作微信消息主表。

## 9. 完整读取流程

### 9.1 步骤

1. 输入会话 ID；
2. 计算 `Msg_<md5>`；
3. 遍历 `message_*.db`；
4. 找到表；
5. 读 `Name2Id`；
6. 读消息；
7. 解压 `source`/`message_content`；
8. 拆 `local_type`；
9. 解析 XML；
10. 输出。

### 9.2 可运行代码

```python
import hashlib
import sqlite3
from pathlib import Path

def quote(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'

def find_table(conn, conversation_id: str):
    wanted = "Msg_" + hashlib.md5(conversation_id.encode()).hexdigest()
    rows = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'Msg_%'"
    )
    for (name,) in rows:
        if name.lower() == wanted.lower():
            return name
    return None

def read_messages(db_storage: str, conversation_id: str, limit: int = 100):
    root = Path(db_storage)
    for db_path in sorted((root / "message").glob("*.db")):
        if db_path.name.endswith("_fts.db") or db_path.name == "message_resource.db":
            continue
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        try:
            table = find_table(conn, conversation_id)
            if not table:
                continue
            names = dict(conn.execute("SELECT rowid, user_name FROM Name2Id"))
            sql = f"""
                SELECT local_id, server_id, local_type, sort_seq, real_sender_id,
                       create_time, source, message_content, compress_content,
                       WCDB_CT_message_content, WCDB_CT_source
                FROM {quote(table)}
                ORDER BY create_time, sort_seq, local_id
                LIMIT ?
            """
            for row in conn.execute(sql, (limit,)):
                yield db_path.name, table, names, row
        finally:
            conn.close()

for db_name, table, names, row in read_messages(
    r"C:\Users\你\Downloads\db_decrypted", "123456789@chatroom", 20
):
    print(db_name, table, row[0], names.get(row[4], row[4]))
```

更完整的解码代码见：

- [docs/examples/read_messages.py](../examples/read_messages.py)
- [WCDB 总文档第 20 章](../WCDB.md)

## 10. 消息分片与去重

### 10.1 一个会话可能跨分片

不要只读 `message_0.db`。正确做法：

```python
for path in root.joinpath("message").glob("*.db"):
    # 每个分片都尝试找 Msg_<hash>
    ...
```

### 10.2 去重

不同分片可能存在重复/迁移记录。可以用：

```text
server_id
```

优先去重；没有 server_id 时用：

```text
(local_id, create_time, real_sender_id, message_content 的哈希)
```

### 10.3 排序

推荐：

```sql
ORDER BY create_time, sort_seq, local_id
```

不要只按 `local_id` 排序：不同分片的 local_id 不是全局顺序。

## 11. 媒体与语音

消息主表只保存媒体引用：

- `message_content` XML 中的 `<img>` / `<videomsg>` / `<appattach>` / `<emoji>`；
- `packed_info_data` 中的二进制信息；
- `hardlink.db` 中的 MD5 -> 文件路径；
- `media_N.db` 中的语音 BLOB。

详见：

- [04-media.md](04-media.md)
- [08-hardlink.md](08-hardlink.md)
- [12-message-resource.md](12-message-resource.md)

## 12. 恢复注意

单会话备份/恢复时，不能只恢复 `Msg_*`：

1. 恢复消息主表；
2. 恢复 `Name2Id` 映射；
3. 恢复 `SessionTable`，否则会话列表可能不显示；
4. 恢复 FTS 或重建搜索索引，否则搜索缺失；
5. 媒体文件需要单独处理。

本项目命令：

```bash
wechat-exporter conversation --export --backup \
  --key <KEY> -c wxid_xxx -o /path/to/backup

wechat-exporter recovery --input /path/to/backup/backup.zip \
  --target-database /path/to/db_storage --key <KEY> --force
```

## 13. 常见问题

### 13.1 找不到 Msg 表

- 会话 ID 写错；
- MD5 编码不是 UTF-8；
- 消息在 `biz_message_N.db`；
- 该会话没有消息。

### 13.2 消息内容是 XML

这是 appmsg/引用/文件/图片的常见情况。不要直接展示整段 XML，先解析 `<title>`、`<refermsg>`、`<type>`。

### 13.3 引用内容是空的

引用消息的正文可能在：

```text
<refermsg><content>...</content></refermsg>
```

而回复正文在：

```text
<title>...</title>
```

两者要分别取。

### 13.4 语音是 BLOB

`message_content` 里只有语音元数据；真实语音在 `VoiceInfo.voice_data`，且是 Silk。

### 13.5 消息时间对不上

- `create_time` 是秒；
- `sort_seq` 可能是毫秒级顺序；
- 不要混用。

### 13.6 恢复后消息没有出现在微信

微信不是只读 `Msg_*`，还需要：

- `SessionTable`；
- `Name2Id`；
- 媒体/资源；
- 正确的加密 key/salt；
- 微信未运行且文件未被锁。

建议使用本项目的 `recovery`，而不是手工 INSERT。

## 14. 速查

```text
message/message_N.db
├── Msg_<md5>                    聊天消息
├── Name2Id                     发送者映射
├── TimeStamp                   快照时间
├── DeleteInfo / DeleteResInfo   删除信息
├── SendInfo                     发送信息
├── History*Info                 历史/系统消息
└── MessageGroupTimeInfo         群时间信息（部分分片）

message/biz_message_N.db          业务消息，Msg_ 结构相同
```

最常用 SQL：

```sql
SELECT local_id, server_id, local_type, create_time,
       real_sender_id, message_content, WCDB_CT_message_content
FROM "Msg_xxx"
ORDER BY create_time, sort_seq, local_id
LIMIT 50;
```

最重要的三个坑：

1. `local_type` 要拆 64 位；
2. `WCDB_CT_* == 4` 要 zstd；
3. `<patMsg>` 不是拍一拍。
