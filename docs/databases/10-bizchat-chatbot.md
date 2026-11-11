# bizchat / chatbot：企业微信与聊天机器人

## 1. 两个库分别做什么

### bizchat/bizchat.db

企业微信、客服、品牌会话的资料：

- 企业/品牌用户；
- 客服群；
- 群成员；
- 品牌资料。

本次样本 4 张表，但部分为空。

### chatbot/chatbot_message.db

微信“聊天机器人”/对话型机器人的消息和会话：

- 机器人会话；
- 机器人消息；
- 卡片；
- 排序信息。

本次样本 4 张表，当前为空。

## 2. bizchat/bizchat.db

### 2.1 表清单

| 表 | 行数 | 作用 |
|---|---:|---|
| `chat_group` | 0 | 企业/客服群 |
| `my_user_info` | 1 | 当前用户 |
| `name2id` | 4 | 内部 ID 映射 |
| `user_info` | 4 | 企业/品牌用户 |

### 2.2 chat_group

```sql
CREATE TABLE chat_group(
    id INTEGER PRIMARY KEY,
    group_id TEXT,
    brand_user_name TEXT,
    type INTEGER,
    version INTEGER,
    bit_flag INTEGER,
    max_member_count INTEGER,
    chat_name TEXT,
    owner_id TEXT,
    head_img_url TEXT,
    user_list TEXT,
    add_member_url TEXT,
    reserved0 INTEGER,
    reserved1 INTEGER,
    reserved2 TEXT,
    reserved3 TEXT
);
```

字段：

| 字段 | 作用 |
|---|---|
| `group_id` | 群/客服组 ID |
| `brand_user_name` | 品牌/企业用户名 |
| `type` | 类型 |
| `max_member_count` | 最大成员数 |
| `chat_name` | 显示名 |
| `owner_id` | 所有者 |
| `head_img_url` | 头像 |
| `user_list` | 成员列表，可能是 JSON/XML |
| `add_member_url` | 加人 URL |
| `reserved*` | 保留字段 |

### 2.3 my_user_info

```sql
CREATE TABLE my_user_info(
    brand_user_name TEXT PRIMARY KEY,
    user_id TEXT
);
```

当前登录企业微信用户。

### 2.4 user_info

```sql
CREATE TABLE user_info(
    id INTEGER PRIMARY KEY,
    user_id TEXT,
    brand_user_name TEXT,
    user_name TEXT,
    version INTEGER,
    bit_flag INTEGER,
    head_img_url TEXT,
    profile_url TEXT,
    add_member_url TEXT,
    reserved0 INTEGER,
    reserved1 INTEGER,
    reserved2 TEXT,
    reserved3 TEXT
);
```

企业成员/客服资料。

### 2.5 name2id

```sql
CREATE TABLE name2id(
    username TEXT PRIMARY KEY
);
```

内部映射。

### 2.6 Python：查询企业用户

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\bizchat\bizchat.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

for row in conn.execute("""
    SELECT id, user_id, brand_user_name, user_name, head_img_url
    FROM user_info
    LIMIT 20
"""):
    print(row)

for row in conn.execute("SELECT id, group_id, chat_name, owner_id FROM chat_group LIMIT 20"):
    print(row)
```

## 3. chatbot/chatbot_message.db

### 3.1 表清单

| 表 | 作用 |
|---|---|
| `chatbot_message` | 机器人消息 |
| `chatbot_message_v1` | 机器人消息 v1 |
| `chatbot_session` | 机器人会话 |
| `chatbot_sort_info` | 排序信息 |

### 3.2 chatbot_message

```sql
CREATE TABLE chatbot_message(
    svrid INTEGER,
    create_time INTEGER,
    dialogue_id INTEGER,
    ui_state_id INTEGER DEFAULT 0,
    app_ui_state INTEGER DEFAULT 0,
    direction INTEGER DEFAULT 0,
    type INTEGER DEFAULT 0,
    trace_msgid TEXT,
    card_wraplist_buffer BLOB,
    extra_info_buffer BLOB,
    bypass_info_buffer BLOB
);
```

| 字段 | 作用 |
|---|---|
| `svrid` | 服务端消息 ID |
| `create_time` | Unix 秒 |
| `dialogue_id` | 对话 ID |
| `ui_state_id` | UI 状态 |
| `app_ui_state` | App UI 状态 |
| `direction` | 方向（发送/接收） |
| `type` | 消息类型 |
| `trace_msgid` | 追踪 ID |
| `card_wraplist_buffer` | 卡片 BLOB |
| `extra_info_buffer` | 扩展 BLOB |
| `bypass_info_buffer` | 旁路信息 BLOB |

### 3.3 chatbot_message_v1

```sql
CREATE TABLE chatbot_message_v1(
    svrid INTEGER,
    create_time INTEGER,
    dialogue_id INTEGER,
    qa_seq INTEGER,
    ui_state_id INTEGER DEFAULT 0,
    app_ui_state INTEGER DEFAULT 0,
    direction INTEGER DEFAULT 0,
    type INTEGER DEFAULT 0,
    db_sync_seq INTEGER,
    trace_msgid TEXT,
    card_wraplist_buffer BLOB,
    extra_info_buffer BLOB,
    bypass_info_buffer BLOB
);
```

比 `chatbot_message` 多：

```text
qa_seq
db_sync_seq
```

### 3.4 chatbot_session

```sql
CREATE TABLE chatbot_session(
    username TEXT PRIMARY KEY,
    source_type INTEGER DEFAULT 0,
    open_scene INTEGER DEFAULT 0,
    timestamp INTEGER,
    tagid INTEGER DEFAULT 0,
    title TEXT,
    extra_info BLOB
);
```

| 字段 | 作用 |
|---|---|
| `username` | 机器人会话 ID |
| `source_type` | 来源类型 |
| `open_scene` | 打开场景 |
| `timestamp` | Unix 秒 |
| `tagid` | 标签 |
| `title` | 标题 |
| `extra_info` | 扩展 BLOB |

### 3.5 chatbot_sort_info

```sql
CREATE TABLE chatbot_sort_info(
    name TEXT PRIMARY KEY,
    db_seq INTEGER DEFAULT 0,
    qa_seq INTEGER DEFAULT 0,
    timestamp INTEGER DEFAULT 0,
    is_end INTEGER DEFAULT 0
);
```

排序/结束状态。

### 3.6 Python：查询机器人会话

```python
db = Path(r"C:\Users\你\Downloads\db_decrypted\chatbot\chatbot_message.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

for row in conn.execute("""
    SELECT username, title, source_type, timestamp
    FROM chatbot_session
    LIMIT 20
"""):
    print(row)

for row in conn.execute("""
    SELECT svrid, create_time, dialogue_id, direction, type, trace_msgid
    FROM chatbot_message
    ORDER BY create_time DESC
    LIMIT 20
"""):
    print(row)
```

## 4. 实际样本

本次：

```text
bizchat.db:
  chat_group=0
  my_user_info=1
  name2id=4
  user_info=4

chatbot_message.db:
  全部为 0
```

说明当前账号：

- 企业/客服资料较少；
- 机器人库存在但未使用；
- 不能因为表为空就判断文件损坏。

## 5. 常见问题

### 5.1 bizchat 表为空

可能账号没有企业微信/客服数据，不是错误。

### 5.2 chatbot 表为空

可能未使用机器人，或数据在其他版本表。

### 5.3 BLOB 字段怎么读

`card_wraplist_buffer`、`extra_info_buffer`、`bypass_info_buffer` 是二进制，可能是 protobuf。不要按文本解码，先保留原始字节。

### 5.4 user_list 怎么解析

`chat_group.user_list` 可能是 JSON/XML/自定义格式；先打印前 200 字节判断。

## 6. 速查

```text
bizchat/bizchat.db
├── chat_group       企业/客服群
├── my_user_info     当前用户
├── user_info        企业用户
└── name2id          内部映射

chatbot/chatbot_message.db
├── chatbot_message      机器人消息
├── chatbot_message_v1   机器人消息 v1
├── chatbot_session      会话
└── chatbot_sort_info    排序
```

关键点：

1. 空表是正常状态；
2. `*_buffer` 是二进制；
3. `create_time` 是 Unix 秒；
4. `username`/`dialogue_id` 是关联键；
5. 企业微信 ID 常见 `@openim`、`@kefu.openim`。
