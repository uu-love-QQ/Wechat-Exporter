# message/media_N.db：语音与媒体索引

## 1. 这个库是做什么的

`message/media_N.db` 主要保存**语音消息的二进制数据**：

```text
message/media_0.db
message/media_1.db
message/biz_media_0.db
```

本次样本：

| 文件 | 表数 | VoiceInfo 行数 |
|---|---:|---:|
| `message/media_0.db` | 3 | 3839 |
| `message/media_1.db` | 3 | 670 |
| `message/biz_media_0.db` | 3 | 2 |

语音消息在聊天主表里通常只有：

```text
local_type = 34
message_content = [语音消息] 或语音 XML
```

真正的语音 BLOB 在这个库里。

## 2. 实际表结构

三个库结构相同：

```sql
CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY
);

CREATE TABLE TimeStamp(
    timestamp INTEGER
);

CREATE TABLE VoiceInfo(
    chat_name_id INTEGER,
    create_time INTEGER,
    local_id INTEGER,
    svr_id INTEGER,
    voice_data BLOB,
    data_index TEXT DEFAULT '0'
);
```

### 2.1 Name2Id

```sql
CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY
);
```

这里的 `rowid` 对应 `VoiceInfo.chat_name_id`，表示语音属于哪个会话。

注意：media 库的 `Name2Id` 只有 `user_name`，没有 `is_session`。

### 2.2 TimeStamp

```sql
CREATE TABLE TimeStamp(
    timestamp INTEGER
);
```

媒体库快照/更新时间，Unix 秒。

### 2.3 VoiceInfo 字段

| 字段 | 类型 | 含义 |
|---|---|---|
| `chat_name_id` | INTEGER | 指向本库 `Name2Id.rowid`，即会话 |
| `create_time` | INTEGER | 语音时间，Unix 秒 |
| `local_id` | INTEGER | 对应消息的 local_id |
| `svr_id` | INTEGER | 对应消息的 server_id |
| `voice_data` | BLOB | Silk 编码语音 |
| `data_index` | TEXT | 同秒/同消息的多段语音排序 |

## 3. voice_data 到底是什么

实际数据开头：

```text
02 23 21 53 49 4c 4b 5f 56 33 ...
```

转换为文本：

```text
#!SILK_V3
```

所以：

- 它不是 WAV；
- 不是 MP3；
- 不是直接可播放的音频；
- 需要用 Silk 解码器转成 PCM，再封装 WAV。

### 3.1 为什么不能直接写 .wav

WAV 需要：

```text
RIFF 头
fmt  块
data 块
PCM 数据
```

`voice_data` 只有 Silk 编码数据，缺少这些结构。

### 3.2 本项目已处理

`wechat-exporter` 会自动：

```text
VoiceInfo.voice_data (Silk)
  -> Silk 解码
  -> PCM
  -> WAV
```

导出后的文件通常是：

```text
media/voice/<hash>.wav
```

## 4. 如何匹配消息和语音

语音消息在主消息表里：

```text
local_id
server_id (svr_id)
create_time
```

在 `VoiceInfo` 里也有：

```text
local_id
svr_id
create_time
```

匹配优先级建议：

1. `svr_id` 相同且 `chat_name_id` 对应同一会话；
2. `local_id + create_time`；
3. 同秒多段语音用 `data_index` 排序。

### 4.1 先查会话的 Name2Id

```python
import sqlite3
from pathlib import Path

media_db = Path(r"C:\Users\你\Downloads\db_decrypted\message\media_0.db")
conn = sqlite3.connect(f"file:{media_db}?mode=ro", uri=True)

id_to_name = dict(conn.execute("SELECT rowid, user_name FROM Name2Id"))
print(id_to_name)
```

### 4.2 查某个会话的语音

```python
conversation_id = "123456789@chatroom"

# 先找 chat_name_id
row = conn.execute(
    "SELECT rowid FROM Name2Id WHERE user_name = ?1",
    (conversation_id,),
).fetchone()

if row:
    chat_name_id = row[0]
    for voice in conn.execute(
        """
        SELECT local_id, svr_id, create_time, data_index, length(voice_data)
        FROM VoiceInfo
        WHERE chat_name_id = ?
        ORDER BY create_time, data_index
        LIMIT 20
        """,
        (chat_name_id,),
    ):
        print(voice)
```

### 4.3 导出原始 Silk

```python
import sqlite3
from pathlib import Path

media_db = Path(r"C:\Users\你\Downloads\db_decrypted\message\media_0.db")
output = Path("voice.silk")

conn = sqlite3.connect(f"file:{media_db}?mode=ro", uri=True)
row = conn.execute(
    "SELECT voice_data FROM VoiceInfo WHERE svr_id = ?1 LIMIT 1",
    ("1234567890123456789",),
).fetchone()

if row and row[0]:
    output.write_bytes(row[0])
    print("已写出", output, len(row[0]), "字节")
```

写出 `.silk` 后可以交给 Silk 解码工具；普通播放器不能直接播放。

### 4.4 检查 Silk 头

```python
data = row[0]
print(data[:10])
# b'\x02#!SILK_V3'
```

有些数据前面会有一个 `0x02` 字节，后面才是 `#!SILK_V3`；解码器通常能识别。

## 5. 媒体库和消息库的关系

```text
message/message_N.db
  Msg_<hash>
    local_id, server_id, create_time, local_type=34
          |
          | 匹配
          v
message/media_N.db
  VoiceInfo
    local_id, svr_id, create_time, chat_name_id
```

`chat_name_id` 不是 wxid，而是该媒体库 `Name2Id.rowid`。

## 6. 图片、视频、文件在哪里

`media_N.db` 本次只包含语音。图片/视频/文件通常在：

```text
<账号目录>/msg/attach/<...>/Img/
<账号目录>/msg/video/
<账号目录>/FileStorage/File/
```

以及：

```text
message/message_resource.db
hardlink/hardlink.db
```

消息主表只保存 MD5/URL/文件名，需要结合：

- [08-hardlink.md](08-hardlink.md)
- [12-message-resource.md](12-message-resource.md)

## 7. 实际样本分析

本次 `media_0.db` 中：

```text
VoiceInfo 行数：3839
voice_data 开头：02 23 21 53 49 4c 4b 5f 56 33
data_index：TEXT，例如 "0"
Name2Id：85 个会话
```

`media_1.db`：

```text
VoiceInfo 行数：670
Name2Id：25 个会话
```

`biz_media_0.db`：

```text
VoiceInfo 行数：2
Name2Id：2 个会话
```

## 8. 常见问题

### 8.1 VoiceInfo 有数据但播放不了

- 它是 Silk，不是 WAV；
- 需要 Silk 解码；
- 不要只改扩展名。

### 8.2 找不到语音

- 语音可能在 `media_1.db`；
- `chat_name_id` 映射在其他库；
- 语音消息的 `svr_id` 可能为 0；
- 尝试用 `local_id + create_time` 匹配。

### 8.3 data_index 是 TEXT

实际是 `'0'` 这样的字符串，不是整数。排序时可以转 int：

```python
int(data_index or 0)
```

### 8.4 Name2Id 和消息库的 Name2Id 不一样

media 库的 `Name2Id` 只有 `user_name`；不要拿消息库的 `is_session` 版本直接套。

## 9. 速查

```text
message/media_N.db
├── Name2Id     rowid -> 会话 wxid
├── TimeStamp   快照时间
└── VoiceInfo
    ├── chat_name_id -> Name2Id.rowid
    ├── local_id
    ├── svr_id
    ├── create_time
    ├── voice_data  Silk BLOB
    └── data_index  TEXT
```

关键点：

1. `voice_data` 是 Silk；
2. 需要解码成 PCM/WAV；
3. `chat_name_id` 只在本库有效；
4. 匹配消息用 `svr_id`/`local_id`/`create_time`；
5. 图片/视频/文件不在这个库，去 `hardlink` 和 `message_resource`。
