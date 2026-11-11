# general/general.db：转账、红包、撤回、搜索等杂项

## 1. 这个库是做什么的

`general/general.db` 是微信的“杂项状态库”，保存：

- 转账；
- 红包；
- 群收款；
- 撤回消息；
- 好友请求；
- 搜索记录；
- 转发记录；
- 视频号状态；
- 小程序/公众号联系人；
- 青少年模式；
- 其他全局状态。

它不是聊天主库，很多表只保存辅助状态。

## 2. 实际表清单

本次样本 24 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `FMessageTable` | 44 | 好友请求/系统通知 |
| `ForwardRecent` | 45 | 最近转发 |
| `SearchRecent` | 19 | 最近搜索 |
| `WeAppBizAttrSyncBufferTableV02` | 504 | 小程序属性同步 |
| `biz_pay_status` | 0 | 公众号支付状态 |
| `biz_subscribe_status` | 33 | 公众号订阅状态 |
| `brand_search_record` | 2 | 品牌搜索 |
| `groupPayTable` | 5 | 群收款 |
| `handoff_remind_v0` | 0 | 接力提醒 |
| `ilink_voip` | 0 | 语音通话 |
| `new_tips` | 0 | 新提示 |
| `redEnvelopeTable` | 90 | 红包 |
| `reddot` | 0 | 红点 |
| `reddot_last_notify` | 0 | 红点通知 |
| `reddot_record` | 3 | 红点记录 |
| `revokebatchmessage` | 0 | 批量撤回 |
| `revokemessage` | 0 | 撤回 |
| `teenager_apply_access_agree_info` | 0 | 青少年模式 |
| `transferTable` | 226 | 转账 |
| `wacontact` | 504 | 小程序/公众号联系人 |
| `wcfinderlivestatus` | 14 | 视频号直播状态 |
| `wcfinderuserpage` | 42 | 视频号主页 |
| `websearch_record` | 11 | 搜索记录 |
| `sqlite_sequence` | 1 | 自增 |

## 3. 转账：transferTable

```sql
CREATE TABLE transferTable(
    transfer_id TEXT PRIMARY KEY,
    transcation_id TEXT,
    message_server_id INTEGER,
    second_message_server_id INTEGER,
    session_name TEXT,
    pay_sub_type INTEGER,
    pay_receiver TEXT,
    pay_payer TEXT,
    begin_transfer_time INTEGER,
    last_modified_time INTEGER,
    invalid_time INTEGER,
    last_update_time INTEGER,
    delay_confirm_flag INTEGER,
    bubble_clicked_flag INTEGER
);
```

| 字段 | 作用 |
|---|---|
| `transfer_id` | 转账 ID |
| `transcation_id` | 交易 ID（微信就是这么拼的，不是 typo） |
| `message_server_id` | 对应消息 server_id |
| `second_message_server_id` | 第二条消息 server_id |
| `session_name` | 会话 |
| `pay_sub_type` | 支付子类型 |
| `pay_receiver` | 收款人 |
| `pay_payer` | 付款人 |
| `begin_transfer_time` | 开始时间 |
| `last_modified_time` | 最后修改 |
| `invalid_time` | 失效时间 |
| `last_update_time` | 最后更新 |
| `delay_confirm_flag` | 延迟确认 |
| `bubble_clicked_flag` | 气泡点击状态 |

### 3.1 Python：列出转账

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\general\general.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

sql = """
SELECT transfer_id, session_name, pay_payer, pay_receiver,
       begin_transfer_time, last_update_time
FROM transferTable
ORDER BY begin_transfer_time DESC
LIMIT 20
"""
for row in conn.execute(sql):
    print(row)
```

## 4. 红包：redEnvelopeTable

```sql
CREATE TABLE redEnvelopeTable(
    message_server_id INTEGER PRIMARY KEY,
    session_name TEXT,
    sender_user_name TEXT,
    native_url TEXT,
    send_id TEXT,
    scene_id INTEGER,
    hb_status INTEGER,
    hb_type INTEGER,
    receive_status INTEGER
);
```

| 字段 | 作用 |
|---|---|
| `message_server_id` | 消息 server_id |
| `session_name` | 会话 |
| `sender_user_name` | 发送者 |
| `native_url` | 红包原始 URL |
| `send_id` | 发送 ID |
| `scene_id` | 场景 |
| `hb_status` | 红包状态 |
| `hb_type` | 红包类型 |
| `receive_status` | 领取状态 |

### 4.1 Python：列出红包

```python
sql = """
SELECT message_server_id, session_name, sender_user_name,
       hb_status, hb_type, receive_status
FROM redEnvelopeTable
ORDER BY message_server_id DESC
LIMIT 20
"""
for row in conn.execute(sql):
    print(row)
```

## 5. 群收款：groupPayTable

```sql
CREATE TABLE groupPayTable(
    bill_no TEXT PRIMARY KEY,
    session_name TEXT,
    message_local_id INTEGER,
    message_create_time INTEGER
);
```

| 字段 | 作用 |
|---|---|
| `bill_no` | 账单号 |
| `session_name` | 群 |
| `message_local_id` | 消息 local_id |
| `message_create_time` | 消息时间 |

## 6. 撤回：revokemessage / revokebatchmessage

### 6.1 revokemessage

```sql
CREATE TABLE revokemessage(
    to_user_name TEXT,
    svr_id INTEGER,
    message_type INTEGER,
    revoke_time INTEGER,
    content TEXT,
    at_user_list TEXT
);
```

### 6.2 revokebatchmessage

```sql
CREATE TABLE revokebatchmessage(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER,
    msg_unique_id TEXT,
    session_name TEXT,
    msg_local_id INTEGER,
    msg_create_time INTEGER
);
```

`at_user_list` 可能是 JSON/逗号分隔；`content` 可能是 XML。

### 6.3 Python：列表撤回记录

```python
for row in conn.execute("""
    SELECT to_user_name, svr_id, message_type, revoke_time, content
    FROM revokemessage
    ORDER BY revoke_time DESC
    LIMIT 20
"""):
    print(row)
```

注意：实际聊天里的“撤回了一条消息”也可能在 `message_N.db` 的系统消息中，`general.db` 只是辅助记录。

## 7. 好友请求：FMessageTable

```sql
CREATE TABLE FMessageTable(
    user_name_ TEXT,
    type_ INTEGER,
    timestamp_ INTEGER,
    encrypt_user_name_ TEXT,
    content_ TEXT,
    is_sender_ INTEGER,
    ticket_ TEXT,
    scene_ INTEGER,
    fmessage_detail_buf_ TEXT,
    remark_ TEXT,
    label_ids_ TEXT
);
```

| 字段 | 作用 |
|---|---|
| `user_name_` | 对方 wxid |
| `type_` | 请求类型 |
| `timestamp_` | 时间 |
| `encrypt_user_name_` | 加密用户名 |
| `content_` | 验证内容 |
| `is_sender_` | 是否我发出 |
| `ticket_` | 请求 ticket |
| `scene_` | 来源场景 |
| `fmessage_detail_buf_` | 详情 |
| `remark_` | 备注 |
| `label_ids_` | 标签 |

### 7.1 Python：好友请求

```python
for row in conn.execute("""
    SELECT user_name_, type_, timestamp_, content_, remark_
    FROM FMessageTable
    ORDER BY timestamp_ DESC
    LIMIT 20
"""):
    print(row)
```

## 8. 搜索记录：SearchRecent / websearch_record

### 8.1 SearchRecent

```sql
CREATE TABLE SearchRecent(
    username TEXT,
    query TEXT,
    score INTEGER,
    last_click_time INTEGER,
    CONSTRAINT _query_username UNIQUE(query, username)
);
```

### 8.2 websearch_record

```sql
CREATE TABLE websearch_record(
    keyword TEXT PRIMARY KEY,
    pay_load_ TEXT,
    create_time INTEGER
);
```

`pay_load_` 可能是 JSON/XML，不要直接当纯文本展示。

### 8.3 Python：最近搜索

```python
for row in conn.execute("""
    SELECT query, username, last_click_time
    FROM SearchRecent
    ORDER BY last_click_time DESC
    LIMIT 20
"""):
    print(row)
```

## 9. 视频号：wcfinder*

### 9.1 wcfinderlivestatus

```sql
CREATE TABLE wcfinderlivestatus(
    finder_live_id INTEGER,
    finder_username TEXT,
    finder_export_id TEXT,
    live_status INTEGER,
    replay_status INTEGER,
    charge_flag INTEGER
);
```

### 9.2 wcfinderuserpage

```sql
CREATE TABLE wcfinderuserpage(
    username TEXT,
    extra_buffer BLOB
);
```

`extra_buffer` 是二进制，通常是 protobuf。

### 9.3 Python：视频号直播状态

```python
for row in conn.execute("""
    SELECT finder_username, live_status, replay_status, charge_flag
    FROM wcfinderlivestatus
    LIMIT 20
"""):
    print(row)
```

## 10. 小程序/公众号联系人：wacontact

```sql
CREATE TABLE wacontact(
    user_name TEXT PRIMARY KEY,
    type INTEGER,
    brand_icon_url TEXT,
    external_info TEXT,
    contact_pack_data BLOB,
    wx_app_opt INTEGER,
    head_image_status TEXT,
    app_id TEXT
);
```

| 字段 | 作用 |
|---|---|
| `user_name` | 用户名 |
| `type` | 类型 |
| `brand_icon_url` | 图标 |
| `external_info` | 外部信息 |
| `contact_pack_data` | 打包数据 BLOB |
| `wx_app_opt` | 小程序选项 |
| `head_image_status` | 头像状态 |
| `app_id` | AppID |

## 11. 其他表

### 11.1 ForwardRecent

```sql
CREATE TABLE ForwardRecent(
    username TEXT,
    forward_time INTEGER
);
```

最近转发对象。

### 11.2 WeAppBizAttrSyncBufferTableV02

```sql
CREATE TABLE WeAppBizAttrSyncBufferTableV02(
    user_name TEXT PRIMARY KEY,
    last_update_time INTEGER,
    version TEXT
);
```

小程序属性同步。

### 11.3 biz_pay_status

```sql
CREATE TABLE biz_pay_status(
    url_id TEXT,
    is_charge_appmsg INTEGER,
    is_paid INTEGER,
    friend_pay_count_str TEXT
);
```

### 11.4 biz_subscribe_status

```sql
CREATE TABLE biz_subscribe_status(
    biz_username TEXT,
    template_id TEXT,
    is_subscribe INTEGER
);
```

### 11.5 brand_search_record

```sql
CREATE TABLE brand_search_record(
    keyword TEXT PRIMARY KEY,
    pay_load_ TEXT,
    create_time INTEGER
);
```

### 11.6 handoff_remind_v0

```sql
CREATE TABLE handoff_remind_v0(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT,
    head_icon TEXT,
    title TEXT,
    desc_type TEXT,
    create_time INTEGER,
    start_time INTEGER,
    expire_time INTEGER,
    biz_type INTEGER,
    version INTEGER,
    url TEXT,
    extra_info TEXT
);
```

### 11.7 ilink_voip

```sql
CREATE TABLE ilink_voip(
    wx_chatroom_ TEXT PRIMARY KEY,
    millsecond_ INTEGER,
    group_id_ TEXT,
    room_id_ INTEGER,
    room_key_ INTEGER,
    route_id_ INTEGER,
    voice_status_ INTEGER,
    talker_create_user_ TEXT,
    not_friend_user_list_ TEXT,
    members_ TEXT,
    is_ilink_ INTEGER,
    ever_quit_chatroom_ INTEGER
);
```

### 11.8 new_tips

```sql
CREATE TABLE new_tips(
    unique_id TEXT PRIMARY KEY,
    disable INTEGER,
    new_tips_content TEXT
);
```

### 11.9 reddot / reddot_last_notify / reddot_record

```sql
CREATE TABLE reddot(
    tips_uuid TEXT PRIMARY KEY,
    tips_content TEXT
);

CREATE TABLE reddot_last_notify(
    tips_uuid TEXT PRIMARY KEY,
    tips_content TEXT
);

CREATE TABLE reddot_record(
    business_type INTEGER PRIMARY KEY,
    record_content TEXT
);
```

### 11.10 teenager_apply_access_agree_info

```sql
CREATE TABLE teenager_apply_access_agree_info(
    access_content_key TEXT PRIMARY KEY,
    access_content_type INTEGER,
    agree_time INTEGER
);
```

## 12. 常见问题

### 12.1 general.db 表很多但大多为空

正常。它保存全局状态，不是所有功能都使用。

### 12.2 转账/红包只有一条消息 ID

需要通过 `message_server_id` 去消息表关联。

### 12.3 时间字段单位

`*_time`、`timestamp_` 通常是 Unix 秒；个别可能是毫秒，按数量级判断。

### 12.4 撤回记录和消息表中的撤回

两者可能都存在：

- 消息表里的系统消息是聊天记录的一部分；
- `revokemessage` 是辅助状态；
- 不要重复统计。

## 13. 速查

```text
general/general.db
├── transferTable          转账
├── redEnvelopeTable       红包
├── groupPayTable          群收款
├── revokemessage          撤回
├── FMessageTable          好友请求
├── SearchRecent           搜索
├── wacontact              小程序/公众号
├── wcfinder*              视频号
└── ...                    其他状态
```

关键点：

1. 它主要是辅助状态，不是聊天主库；
2. 转账/红包用 `message_server_id` 关联消息；
3. 时间通常是 Unix 秒；
4. `pay_load_`/`content_` 可能是 JSON/XML；
5. 空表是正常现象。
