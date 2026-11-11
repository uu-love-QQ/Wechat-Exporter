# contact/contact.db：联系人、群、标签、公众号

## 1. 这个库是做什么的

`contact/contact.db` 是微信的“通讯录”：

- 好友；
- 群聊；
- 群成员关系；
- 公众号/服务号；
- 企业微信联系人；
- 陌生人和验证申请；
- 联系人标签；
- 公众号资料。

它不保存聊天消息。聊天消息在 `message/message_N.db`。

## 2. 实际表清单

本次样本 17 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `contact` | 14532 | 好友、群、公众号等主表 |
| `stranger` | 6 | 陌生人/非好友 |
| `chat_room` | 139 | 群资料 |
| `chat_room_info_detail` | 139 | 群公告、群状态 |
| `chatroom_member` | 11989 | 群成员关系 |
| `contact_label` | 6 | 联系人标签 |
| `biz_info` | 578 | 公众号/服务号资料 |
| `name2id` | 14561 | 联系人内部 ID 映射 |
| `encrypt_name2id` | 6 | 加密用户名映射 |
| `ticket_info` | 271 | 联系人 ticket |
| `stranger_ticket_info` | 0 | 陌生人 ticket |
| `openim_appid` | 1 | 企业微信 appid |
| `openim_acct_type` | 1 | 企业微信账号类型 |
| `openim_wording` | 62 | 企业微信文案 |
| `oplog` | 0 | 操作日志 |
| `room_verify_application` | 0 | 入群申请 |
| `sqlite_sequence` | 1 | SQLite 自增序列 |

## 3. contact 主表

### 3.1 实际 DDL

```sql
CREATE TABLE contact(
    id INTEGER PRIMARY KEY,
    username TEXT,
    local_type INTEGER,
    alias TEXT,
    encrypt_username TEXT,
    flag INTEGER,
    delete_flag INTEGER,
    verify_flag INTEGER,
    remark TEXT,
    remark_quan_pin TEXT,
    remark_pin_yin_initial TEXT,
    nick_name TEXT,
    pin_yin_initial TEXT,
    quan_pin TEXT,
    big_head_url TEXT,
    small_head_url TEXT,
    head_img_md5 TEXT,
    chat_room_notify INTEGER,
    is_in_chat_room INTEGER,
    description TEXT,
    extra_buffer BLOB,
    chat_room_type INTEGER
);
```

### 3.2 字段解释

| 字段 | 说明 |
|---|---|
| `id` | 联系人内部 ID，群成员表会引用它 |
| `username` | 微信 ID：`wxid_xxx`、`xxx@chatroom`、`gh_xxx`、`xxx@openim` |
| `local_type` | 联系人类型位掩码 |
| `alias` | 微信号 |
| `encrypt_username` | 加密 ID |
| `flag` | 联系人标志位 |
| `delete_flag` | 是否已删除 |
| `verify_flag` | 验证状态 |
| `remark` | 备注名 |
| `remark_quan_pin` | 备注全拼 |
| `remark_pin_yin_initial` | 备注拼音首字母 |
| `nick_name` | 昵称 |
| `pin_yin_initial` | 昵称拼音首字母 |
| `quan_pin` | 昵称全拼 |
| `big_head_url` / `small_head_url` | 头像 URL |
| `head_img_md5` | 头像 MD5 |
| `chat_room_notify` | 群通知；实际是 INTEGER，不是 TEXT |
| `is_in_chat_room` | 是否仍在群内 |
| `description` | 描述 |
| `extra_buffer` | 二进制扩展，常见 protobuf |
| `chat_room_type` | 群类型 |

### 3.3 local_type 实际含义

本次 14532 行的分布：

```text
local_type=3    13037 普通好友
local_type=1      724 公众号/系统/特殊联系人
local_type=0      437 陌生人/未归类
local_type=2      141 群聊
local_type=6      141 企业微信服务/助手
local_type=5       44 企业微信联系人
local_type=7        7 企业客服
local_type=4        1 冻结/特殊公众号
```

实际样本：

```text
local_type=2  -> 43055999748@chatroom
local_type=3  -> wxid_xxx（普通好友）
local_type=5  -> 25984982365661657@openim
local_type=6  -> 25984985289643688@openim
local_type=7  -> 25984993408582700@kefu.openim
local_type=1  -> gh_xxx（公众号）
```

小白建议：

- 判断群聊：`username.endswith("@chatroom")` 最稳；
- 判断公众号：`username.startswith("gh_")`；
- 判断企业微信：`username.endswith("@openim")`；
- `local_type` 只作为辅助。

### 3.4 flag 实际含义

本次样本常见：

```text
flag=4        普通好友
flag=3        公众号/系统号
flag=2050     群聊
flag=513/1    企业微信
flag=0        陌生人
```

`flag & 8` 在本仓库的联系人分类中用于判断黑名单，但不同版本位定义可能不同，建议结合用户名和 `delete_flag` 判断。

### 3.5 Python：列出全部好友

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\contact\contact.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

sql = """
SELECT username, remark, nick_name, alias, local_type, flag
FROM contact
WHERE username NOT LIKE '%@chatroom'
  AND username NOT LIKE 'gh_%'
  AND username NOT LIKE '%@openim'
ORDER BY remark, nick_name
LIMIT 50
"""

for username, remark, nick, alias, local_type, flag in conn.execute(sql):
    display = remark or nick or alias or username
    print(f"{display:20} {username:35} local_type={local_type} flag={flag}")

conn.close()
```

### 3.6 Python：列出群聊

```python
sql = """
SELECT username, remark, nick_name, chat_room_type, is_in_chat_room
FROM contact
WHERE username LIKE '%@chatroom'
ORDER BY nick_name
LIMIT 50
"""

for username, remark, nick, chat_type, in_room in conn.execute(sql):
    print(username, remark or nick, "chat_room_type=", chat_type, "in_room=", in_room)
```

### 3.7 Python：搜索联系人

```python
keyword = "张"
sql = """
SELECT username, remark, nick_name, alias
FROM contact
WHERE remark LIKE ?1 OR nick_name LIKE ?1 OR alias LIKE ?1 OR username LIKE ?1
LIMIT 20
"""
for row in conn.execute(sql, (f"%{keyword}%",)):
    print(row)
```

---

## 4. 群相关表

### 4.1 chat_room

```sql
CREATE TABLE chat_room(
    id INTEGER PRIMARY KEY,
    username TEXT,
    owner TEXT,
    ext_buffer BLOB
);
```

| 字段 | 作用 |
|---|---|
| `id` | 群内部 ID，通常等于 `contact.id` |
| `username` | 群 wxid，`xxx@chatroom` |
| `owner` | 群主 wxid |
| `ext_buffer` | 群扩展 protobuf |

### 4.2 chat_room_info_detail

```sql
CREATE TABLE chat_room_info_detail(
    room_id_ INTEGER PRIMARY KEY,
    username_ TEXT,
    announcement_ TEXT,
    announcement_editor_ TEXT,
    announcement_publish_time_ INTEGER,
    chat_room_status_ INTEGER,
    xml_announcement_ TEXT,
    ext_buffer_ BLOB
);
```

| 字段 | 作用 |
|---|---|
| `room_id_` | 群 contact.id |
| `username_` | 群 wxid |
| `announcement_` | 群公告文本 |
| `announcement_editor_` | 公告编辑者 |
| `announcement_publish_time_` | 公告时间，Unix 秒 |
| `chat_room_status_` | 群状态 |
| `xml_announcement_` | 公告 XML |
| `ext_buffer_` | 扩展 BLOB |

### 4.3 chatroom_member

```sql
CREATE TABLE chatroom_member(
    room_id INTEGER,
    member_id INTEGER,
    CONSTRAINT room_member UNIQUE(room_id, member_id)
);
```

实际关系已确认：

```text
chat_room.id == chatroom_member.room_id
contact.id == chatroom_member.member_id
```

不是 wxid 直接对应。

### 4.4 Python：列出群成员

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\contact\contact.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

group_username = "43055999748@chatroom"

sql = """
SELECT c.username, c.remark, c.nick_name, c.alias
FROM chatroom_member m
JOIN contact c ON c.id = m.member_id
WHERE m.room_id = (
    SELECT id FROM contact WHERE username = ?1
)
ORDER BY c.remark, c.nick_name
"""

for username, remark, nick, alias in conn.execute(sql, (group_username,)):
    print(username, remark or nick or alias)

conn.close()
```

### 4.5 Python：群主和群公告

```python
sql = """
SELECT c.username, r.owner, d.announcement_, d.announcement_editor_,
       d.announcement_publish_time_
FROM contact c
LEFT JOIN chat_room r ON r.id = c.id
LEFT JOIN chat_room_info_detail d ON d.room_id_ = c.id
WHERE c.username LIKE '%@chatroom'
LIMIT 20
"""
for row in conn.execute(sql):
    print(row)
```

---

## 5. 陌生人、ticket、标签

### 5.1 stranger

```sql
CREATE TABLE stranger(
    id INTEGER PRIMARY KEY,
    username TEXT,
    local_type INTEGER,
    alias TEXT,
    encrypt_username TEXT,
    flag INTEGER,
    delete_flag INTEGER,
    verify_flag INTEGER,
    remark TEXT,
    remark_quan_pin TEXT,
    remark_pin_yin_initial TEXT,
    nick_name TEXT,
    pin_yin_initial TEXT,
    quan_pin TEXT,
    big_head_url TEXT,
    small_head_url TEXT,
    head_img_md5 TEXT,
    chat_room_notify INTEGER,
    is_in_chat_room INTEGER,
    description TEXT,
    extra_buffer BLOB,
    chat_room_type INTEGER
);
```

字段与 `contact` 基本一样；用于非好友/陌生人。

### 5.2 ticket_info / stranger_ticket_info

```sql
CREATE TABLE ticket_info(
    id INTEGER PRIMARY KEY,
    ticket TEXT
);

CREATE TABLE stranger_ticket_info(
    id INTEGER PRIMARY KEY,
    ticket TEXT
);
```

`ticket` 是联系人相关的内部令牌，不是密码；不要随便公开。

### 5.3 contact_label

```sql
CREATE TABLE contact_label(
    label_id_ INTEGER PRIMARY KEY,
    label_name_ TEXT,
    sort_order_ INTEGER
);
```

微信通讯录标签。

### 5.4 Python：列出标签

```python
for row in conn.execute(
    "SELECT label_id_, label_name_, sort_order_ FROM contact_label ORDER BY sort_order_"
):
    print(row)
```

---

## 6. name2id 与 encrypt_name2id

```sql
CREATE TABLE name2id(
    username TEXT PRIMARY KEY
);

CREATE TABLE encrypt_name2id(
    username TEXT PRIMARY KEY
);
```

- `name2id.rowid` 是内部联系人 ID；
- `chatroom_member.member_id` 更常见的是引用 `contact.id`，不是 `name2id.rowid`；
- `encrypt_name2id` 保存加密用户名映射。

### 6.1 Python：按内部 ID 查联系人

```python
sql = """
SELECT rowid, username FROM name2id LIMIT 10
"""
for rowid, username in conn.execute(sql):
    print(rowid, username)
```

---

## 7. 公众号/服务号：biz_info

```sql
CREATE TABLE biz_info(
    id INTEGER PRIMARY KEY,
    username TEXT,
    type INTEGER,
    accept_type INTEGER,
    child_type INTEGER,
    version INTEGER,
    external_info TEXT,
    brand_info TEXT,
    brand_icon_url TEXT,
    brand_list TEXT,
    brand_flag INTEGER,
    belong TEXT,
    ext_buffer BLOB,
    home_url TEXT,
    sync_version TEXT
);
```

| 字段 | 含义 |
|---|---|
| `username` | 公众号 `gh_xxx` |
| `type` | 公众号类型 |
| `accept_type` | 接收消息类型 |
| `child_type` | 子类型 |
| `external_info` | 外部信息，常见 JSON/XML |
| `brand_info` | 品牌信息 |
| `brand_icon_url` | 品牌图标 |
| `brand_list` | 品牌列表 |
| `brand_flag` | 品牌标志 |
| `belong` | 归属 |
| `ext_buffer` | 二进制扩展 |
| `home_url` | 主页 URL |
| `sync_version` | 同步版本 |

### 7.1 Python：列出公众号

```python
sql = """
SELECT c.username, c.nick_name, c.remark, b.type, b.child_type, b.brand_flag
FROM contact c
LEFT JOIN biz_info b ON b.username = c.username
WHERE c.username LIKE 'gh_%'
ORDER BY c.nick_name
LIMIT 50
"""
for row in conn.execute(sql):
    print(row)
```

### 7.2 公众号类型说明

`local_type=1` + 用户名 `gh_` 表示公众号/服务号；仅靠 `type` 判断容易把系统号也算进去，应结合用户名。

---

## 8. 企业微信/客服

### 8.1 openim_appid

```sql
CREATE TABLE openim_appid(
    lang_id INTEGER,
    app_id TEXT,
    acct_type_id TEXT,
    update_time INTEGER,
    ext_buffer BLOB,
    CONSTRAINT appId_langId PRIMARY KEY(app_id, lang_id)
);
```

### 8.2 openim_acct_type

```sql
CREATE TABLE openim_acct_type(
    lang_id INTEGER,
    acc_type_id TEXT,
    update_time INTEGER,
    ext_buffer BLOB,
    CONSTRAINT accTypeId_langId PRIMARY KEY(acc_type_id, lang_id)
);
```

### 8.3 openim_wording

```sql
CREATE TABLE openim_wording(
    lang_id INTEGER,
    app_id TEXT,
    wording_id TEXT,
    wording TEXT,
    pinyin TEXT,
    quan_pin TEXT,
    update_time INTEGER,
    ext_buffer BLOB,
    CONSTRAINT appId_langId_wordingId PRIMARY KEY(app_id, lang_id, wording_id)
);
```

这些表是企业微信的显示文案、类型和 appid 映射。

### 8.4 Python：企业微信联系人

```python
sql = """
SELECT username, remark, nick_name, local_type, flag
FROM contact
WHERE username LIKE '%@openim'
   OR username LIKE '%@kefu.openim'
ORDER BY local_type, nick_name
LIMIT 50
"""
for row in conn.execute(sql):
    print(row)
```

---

## 9. 入群申请与操作日志

### 9.1 room_verify_application

```sql
CREATE TABLE room_verify_application(
    msg_server_id INTEGER PRIMARY KEY,
    chatroom_name TEXT,
    hash_key TEXT,
    inviter_username TEXT,
    state INTEGER,
    read_status INTEGER,
    data_json TEXT,
    update_time INTEGER
);
```

- `chatroom_name`：群 ID；
- `inviter_username`：邀请人；
- `state`：申请状态；
- `data_json`：详细 JSON；
- `update_time`：Unix 秒。

### 9.2 oplog

```sql
CREATE TABLE oplog(
    id INTEGER PRIMARY KEY ASC AUTOINCREMENT,
    buffer BLOB
);
```

内部操作日志，`buffer` 是 BLOB，不要按文本解析。

---

## 10. 联系人头像

`contact` 表里有：

```text
big_head_url
small_head_url
head_img_md5
```

- URL 可以直接下载；
- 本地头像缓存通常在 `head_image/head_image.db`；
- 详见 [09-head-image.md](09-head-image.md)。

---

## 11. 常见问题

### 11.1 群成员显示 wxid

说明该成员不在 `contact`，或已退群/被删除。可以保留 wxid 作为显示名。

### 11.2 群成员只有 member_id

`member_id` 是 `contact.id`，必须 JOIN：

```sql
SELECT c.username
FROM chatroom_member m
JOIN contact c ON c.id = m.member_id
WHERE m.room_id = ?;
```

### 11.3 公众号显示成好友

用 `username LIKE 'gh_%'` 过滤；不要只看 `local_type=1`，因为自己/系统号也可能是 1。

### 11.4 `chat_room_notify` 不是文本

实际是 `INTEGER`，不要按字符串解析。

### 11.5 联系人重复

`contact` 和 `stranger` 都可能有同名记录；优先使用 `contact`，陌生人再查 `stranger`。

---

## 12. 速查：最常用 SQL

```sql
-- 最近联系人
SELECT username, remark, nick_name
FROM contact
ORDER BY nick_name
LIMIT 20;

-- 群聊
SELECT username, nick_name
FROM contact
WHERE username LIKE '%@chatroom';

-- 群成员
SELECT c.username, c.remark, c.nick_name
FROM chatroom_member m
JOIN contact c ON c.id = m.member_id
WHERE m.room_id = (SELECT id FROM contact WHERE username = ?);

-- 公众号
SELECT c.username, c.nick_name, b.type
FROM contact c
LEFT JOIN biz_info b ON b.username = c.username
WHERE c.username LIKE 'gh_%';
```
