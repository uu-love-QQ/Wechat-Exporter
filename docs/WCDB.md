# 微信 4.x WCDB 数据库与表结构完整摘要

> 本文件基于对 `out/` 中实际 `db_storage`（27 个数据库）的解密采样整理。
> 表结构会随微信小版本、账号类型、是否启用某些功能而变化。
> 文中“实际观察”表示已在本次 `out/` 数据中验证；“版本变体”表示不同版本可能出现。

## 0. 采样数据概况

本次 `out/` 解密后的数据库：

| 数据库 | 表数量（含 FTS/内部表） | 说明 |
|---|---:|---|
| `bizchat/bizchat.db` | 4 | 企业微信/客服资料 |
| `chatbot/chatbot_message.db` | 4 | 聊天机器人 |
| `contact/contact.db` | 17 | 联系人、群、标签、公众号资料 |
| `contact/contact_fts.db` | 37 | 联系人 FTS（含拼音索引） |
| `emoticon/emoticon.db` | 7 | 表情包 |
| `favorite/favorite.db` | 9 | 收藏 |
| `favorite/favorite_fts.db` | 7 | 收藏 FTS |
| `general/general.db` | 24 | 通用设置、转账、红包、搜索记录等 |
| `hardlink/hardlink.db` | 8 | 媒体 MD5 -> 文件名/目录 |
| `head_image/head_image.db` | 1 | 头像缓存 |
| `message/biz_media_0.db` | 3 | 业务语音 |
| `message/biz_message_0.db` | 473 | 业务消息分片 |
| `message/biz_message_1.db` | 10 | 业务消息分片 |
| `message/biz_message_2.db` | 7 | 业务消息分片 |
| `message/media_0.db` | 3 | 语音媒体 |
| `message/media_1.db` | 3 | 语音媒体 |
| `message/message_0.db` | 186 | 私聊/群聊消息分片 |
| `message/message_1.db` | 190 | 消息分片 |
| `message/message_2.db` | 130 | 消息分片 |
| `message/message_3.db` | 116 | 消息分片 |
| `message/message_fts.db` | 60 | 消息 FTS |
| `message/message_resource.db` | 7 | 媒体资源索引 |
| `message/weclaw.db` | 1 | WeClaw 内部表 |
| `session/session.db` | 7 | 会话状态 |
| `sns/sns.db` | 13 | 朋友圈 |
| `solitaire/solitaire.db` | 6 | 群接龙 |
| `third_app_icon/third_app_icon.db` | 1 | 第三方应用图标 |

数据库旁边可能还有：

```text
xxx.db-wal
xxx.db-shm
```

必须与主库一起读取、备份、恢复。

---

## 1. 通用规则

### 1.1 SQLCipher 4

- 算法：AES-256-CBC + HMAC-SHA512；
- 页大小通常 4096；
- 每页尾部保留 80 字节：16 字节 IV + 64 字节 HMAC；
- 每个文件前 16 字节是 salt；
- 用户传入的 64 位 Hex 通常是主通行码；
- 最终 key 通常由 `PBKDF2-HMAC-SHA512(master, salt, 256000)` 派生；
- 同一主通行码在不同数据库上得到不同最终 key；
- 兼容路径也可能接收单库 raw key。

### 1.2 SQLite 动态类型

WCDB 虽然声明 `TEXT`，实际可能存 BLOB。例如实际数据中：

```text
message_content TEXT       # 实际可能为 zstd BLOB
compress_content TEXT      # 实际可能为 zstd BLOB
source TEXT                # 实际可能为 zstd BLOB
```

读取时应通过 SQLite 值类型判断，而不是相信声明类型。

### 1.3 zstd

实际 `out/` 中观察到：

```text
WCDB_CT_source = 4
WCDB_CT_message_content = 4
```

对应内容为：

```text
28 B5 2F FD ...
```

即标准 zstd frame。

规则：

- `WCDB_CT_* == 4` 时按 zstd 解压；
- 同时检查魔数 `28 B5 2F FD`；
- `message_content` 为空时回退 `compress_content`；
- `source` 使用 `WCDB_CT_source`；
- 解压后去掉末尾 NUL；
- 压缩字段必须先按字节处理，再做 UTF-8；
- `compress_content` 在本次账号中全部为空，但其他版本可能使用，必须保留回退；
- `packed_info_data`、`pack_info_buf`、`serialized_*_buf`、`ext_buffer` 是二进制，可能是 protobuf，不能当文本。

### 1.4 `local_type`

```text
base_type = local_type & 0xFFFFFFFF
sub_type  = local_type >> 32
```

实际观察到的值：

```text
1
3
43
47
48
10000
244813135921       = 57 << 32 | 49   引用
266287972401       = 62 << 32 | 49   拍一拍
21474836529        = 5  << 32 | 49   链接/文章
25769803825        = 6  << 32 | 49   文件
219043332145       = 51 << 32 | 49   视频号
81604378673        = 19 << 32 | 49   聊天记录
8594229559345      = 2001 << 32 | 49 等扩展
12884901937        = 3  << 32 | 49
34359738417        = 8  << 32 | 49
```

### 1.5 `sort_seq`

实际消息表中的 `sort_seq` 不是小整数，而是接近 13 位的时间/顺序值，例如：

```text
create_time = 1781533534
sort_seq    = 1781533534000
```

通常可按 `create_time, sort_seq, local_id` 排序；不要假设 `sort_seq` 是 1、2、3 这种小序号。

### 1.6 发送者映射

消息表：

```text
real_sender_id -> 本库 Name2Id.rowid
```

实际 `Name2Id` 结构是：

```sql
CREATE TABLE Name2Id(user_name TEXT PRIMARY KEY, is_session INTEGER);
```

不是固定 `is_group`；不同版本/表可能使用 `is_group`。`is_session=1` 表示该映射对应会话实体，不能简单等同于“群”。

群消息文本常带：

```text
wxid_xxx:
正文
```

该前缀通常比历史 `real_sender_id` 更可靠。

### 1.7 XML

常见字段：

- `message_content`：文本、图片 XML、引用 XML、appmsg XML；
- `source`：msgsource XML；
- `packed_info_data`：二进制媒体信息；
- 收藏 `content`、朋友圈 `content`：XML。

需要处理 XML 声明、CDATA、实体、转义嵌套 XML。

---

## 2. session/session.db

实际表结构：

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

CREATE TABLE SessionDeleteTable(
    username TEXT PRIMARY KEY,
    delete_time INTEGER
);

CREATE TABLE SessionUnreadListTable_1(
    username_id INTEGER,
    server_id INTEGER,
    create_time INTEGER,
    local_id INTEGER,
    CONSTRAINT _NameId_ServerId PRIMARY KEY(username_id, server_id)
);

CREATE TABLE SessionUnreadStatTable_1(
    username_id INTEGER,
    unread_stat INTEGER,
    CONSTRAINT _NameId PRIMARY KEY(username_id)
);

CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY
);

CREATE TABLE SessionNoContactInfoTable(
    username TEXT PRIMARY KEY,
    session_title TEXT
);

CREATE TABLE SessionDraft(
    username TEXT,
    window_id TEXT,
    timestamp INTEGER,
    draft_data BLOB
);
```

索引：

```text
SessionTable_LSENDER(last_msg_sender)
SessionTable_TYPE(type)
SessionUnreadListTable_1_NameId_CreateTime(username_id, create_time)
SessionDraft_SESSION_DRAFT(username, window_id) UNIQUE
```

字段说明：

| 字段 | 作用 |
|---|---|
| `SessionTable.username` | 会话 ID |
| `type` | 会话类型；本次账号中全部为 0，不能依赖它判断群聊 |
| `unread_count` | 未读数 |
| `unread_first_msg_srv_id` | 未读起点 server_id |
| `unread_first_pat_msg_local_id` | 未读拍一拍消息 |
| `unread_first_pat_msg_sort_seq` | 未读拍一拍排序 |
| `is_hidden` | 是否隐藏 |
| `summary` | 最后一条摘要 |
| `draft` | 草稿摘要 |
| `status` | 状态位 |
| `last_timestamp` | 最后消息时间，Unix 秒 |
| `sort_timestamp` | 排序时间，本次账号大量为 0 |
| `last_clear_unread_timestamp` | 清未读时间 |
| `last_msg_locald_id` | 最后消息 local_id |
| `last_msg_type` | 最后消息基础类型 |
| `last_msg_sub_type` | 最后消息子类型 |
| `last_msg_sender` | 最后消息发送者 |
| `last_sender_display_name` | 最后发送者显示名 |
| `last_msg_ext_type` | 扩展类型 |

辅助表：

- `SessionUnreadListTable_1`：每个会话的未读消息列表；`username_id` 指向本库 `Name2Id.rowid`；
- `SessionUnreadStatTable_1`：未读统计；
- `SessionDeleteTable`：删除会话记录；
- `SessionNoContactInfoTable`：无联系人信息时的会话标题；
- `SessionDraft`：会话草稿，`draft_data` 是 BLOB。

---

## 3. contact/contact.db

### 3.1 contact

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

实际观察到的 `local_type`：

| local_type | 样本 | 含义 |
|---:|---|---|
| 0 | `wxid_...` | 陌生人/未归类联系人 |
| 1 | `gh_...`、系统号、自己 | 公众号/系统/特殊联系人 |
| 2 | `...@chatroom` | 群聊 |
| 3 | `wxid_...` | 普通好友 |
| 4 | `gh_...` | 冻结/特殊公众号 |
| 5 | `...@openim` | 企业微信联系人 |
| 6 | `...@openim` | 企业微信服务/助手 |
| 7 | `...@kefu.openim` | 企业客服 |

`flag` 是位掩码；本次样本常见：

```text
4        普通好友
3        公众号/系统号
2050     群聊
513/1    企业微信
0        陌生人
```

`flag & 8` 在本项目的分类逻辑中表示黑名单，但具体位定义随版本变化，建议结合 `local_type`、`delete_flag`、`verify_flag` 和用户名判断。

其他字段：

| 字段 | 作用 |
|---|---|
| `alias` | 微信号 |
| `encrypt_username` | 加密 ID/别名 |
| `delete_flag` | 删除标志 |
| `verify_flag` | 验证状态 |
| `remark` | 备注 |
| `remark_quan_pin` | 备注全拼 |
| `remark_pin_yin_initial` | 备注拼音首字母 |
| `nick_name` | 昵称 |
| `pin_yin_initial` | 昵称拼音首字母 |
| `quan_pin` | 昵称全拼 |
| `big_head_url/small_head_url` | 头像 URL |
| `head_img_md5` | 头像 MD5 |
| `chat_room_notify` | 群通知；实际为 INTEGER，不是 TEXT |
| `is_in_chat_room` | 是否仍在群内 |
| `description` | 描述 |
| `extra_buffer` | 二进制扩展，常见 protobuf |
| `chat_room_type` | 群类型 |

### 3.2 群相关

```sql
CREATE TABLE chat_room(
    id INTEGER PRIMARY KEY,
    username TEXT,
    owner TEXT,
    ext_buffer BLOB
);

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

CREATE TABLE chatroom_member(
    room_id INTEGER,
    member_id INTEGER,
    CONSTRAINT room_member UNIQUE(room_id, member_id)
);
```

实际关系：

```text
chat_room.id == contact.id == chatroom_member.room_id
chatroom_member.member_id == contact.id
```

`chat_room.owner` 是群主 wxid；`ext_buffer` 是 protobuf 二进制群扩展。

`chat_room_info_detail`：

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

### 3.3 其他表

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

`stranger` 与 `contact` 字段基本一致，用于非好友/陌生人。

```sql
CREATE TABLE name2id(username TEXT PRIMARY KEY);
CREATE TABLE encrypt_name2id(username TEXT PRIMARY KEY);
CREATE TABLE ticket_info(id INTEGER PRIMARY KEY, ticket TEXT);
CREATE TABLE stranger_ticket_info(id INTEGER PRIMARY KEY, ticket TEXT);
```

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

`biz_info` 是公众号/服务号资料；`external_info`、`brand_info`、`brand_list` 常为 JSON/XML，`ext_buffer` 为二进制。

```sql
CREATE TABLE contact_label(
    label_id_ INTEGER PRIMARY KEY,
    label_name_ TEXT,
    sort_order_ INTEGER
);

CREATE TABLE openim_appid(
    lang_id INTEGER,
    app_id TEXT,
    acct_type_id TEXT,
    update_time INTEGER,
    ext_buffer BLOB,
    CONSTRAINT appId_langId PRIMARY KEY(app_id, lang_id)
);

CREATE TABLE openim_acct_type(
    lang_id INTEGER,
    acc_type_id TEXT,
    update_time INTEGER,
    ext_buffer BLOB,
    CONSTRAINT accTypeId_langId PRIMARY KEY(acc_type_id, lang_id)
);

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

CREATE TABLE oplog(
    id INTEGER PRIMARY KEY ASC AUTOINCREMENT,
    buffer BLOB
);
```

`oplog.buffer` 是内部操作日志 BLOB，不能按文本处理。

---

## 4. contact/contact_fts.db

这是联系人全文索引，包含 FTS5 虚表和影子表：

```text
chatroom_member_fts_v3
contact_fts_v5
contact_fts_pinyin_v5
search_dict_fts_v1
wa_contact_fts_v1
wa_contact_fts_pinyin_v1
```

实际 DDL 示例：

```sql
CREATE VIRTUAL TABLE chatroom_member_fts_v3
USING fts5(
    tokenize = 'MMFtsTokenizer disable_pinyin enable_special_char',
    a_group_remark,
    room_id UNINDEXED,
    member_id UNINDEXED
);

CREATE VIRTUAL TABLE contact_fts_v5
USING fts5(
    tokenize = 'MMFtsTokenizer disable_pinyin enable_special_char enable_num_token',
    search_key,
    local_type UNINDEXED
);

CREATE VIRTUAL TABLE contact_fts_pinyin_v5
USING fts5(
    tokenize = 'MMFtsTokenizer disable_origin',
    content='contact_fts_v5',
    search_key,
    local_type UNINDEXED
);
```

影子表：

```text
*_config
*_content
*_data
*_docsize
*_idx
```

以及 `chatroom_member_fts_v3_aux`、`search_dict_v1`、`Name2Id`、`db_info`。

注意：

- `MMFtsTokenizer` 是微信自定义 tokenizer；
- 普通 SQLite 读取虚表会报 `no such tokenizer: MMFtsTokenizer`；
- 影子表可以读，但要做全文搜索必须使用微信 tokenizer 或重新构建索引；
- 复制/备份时必须把虚表和全部影子表一起处理。

---

## 5. message/message_N.db

### 5.1 通用消息表

实际 DDL：

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

实际字段位置：

| 字段 | 作用 |
|---|---|
| `local_id` | 会话内本地消息 ID，自增；恢复时按它去重 |
| `server_id` | 服务端消息 ID，常见 19 位，使用 i64/u64 |
| `local_type` | 64 位打包类型 |
| `sort_seq` | 排序/时间序列，实际常见 13 位，如 `1781533534000` |
| `real_sender_id` | 指向本库 `Name2Id.rowid` |
| `create_time` | Unix 秒 |
| `status` | 消息状态 |
| `upload_status` | 上传状态 |
| `download_status` | 下载状态 |
| `server_seq` | 服务端顺序号 |
| `origin_source` | 来源标志 |
| `source` | 来源 XML，常 zstd |
| `message_content` | 主内容，TEXT 或 zstd BLOB |
| `compress_content` | 备用压缩内容；本次为空但需回退 |
| `packed_info_data` | 二进制媒体/扩展数据 |
| `WCDB_CT_message_content` | 内容编码；4 为 zstd |
| `WCDB_CT_source` | source 编码；4 为 zstd |

表名规则：

```text
Msg_<md5(会话 username)>
```

例如 `123456789@chatroom` 对应 `Msg_9234c5fa4065f721b47403f393822c44`。

### 5.2 辅助表

```sql
CREATE TABLE TimeStamp(timestamp INTEGER);

CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY,
    is_session INTEGER
);

CREATE TABLE DeleteInfo(
    chat_name_id INTEGER,
    delete_table_name TEXT,
    CONSTRAINT UNIQUE_CHAT_DELETE UNIQUE(chat_name_id, delete_table_name)
);

CREATE TABLE DeleteResInfo(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_name_id INTEGER,
    msg_create_time INTEGER,
    msg_local_id INTEGER,
    res_path TEXT
);

CREATE TABLE SendInfo(
    chat_name_id INTEGER,
    msg_local_id INTEGER
);

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

说明：

- `DeleteInfo`：记录某会话被删除的表；
- `DeleteResInfo`：删除的资源；
- `SendInfo`：发送信息；
- `History*Info`：历史消息/系统消息索引；
- `TimeStamp.timestamp`：该库快照时间；
- `Name2Id.is_session`：1 表示会话实体，不能等同于 `is_group`。

`message_3.db` 还实际出现：

```sql
CREATE TABLE MessageGroupTimeInfo(
    chatname_id INTEGER,
    group_id TEXT,
    create_time INTEGER,
    initial_sort_seq INTEGER,
    birth_time INTEGER
);
```

`initial_sort_seq` 也是 13 位，`birth_time` 是秒级时间。

### 5.3 非微信原生表

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

CREATE TABLE alpha_message_recall_state_v1(
    conversation_id TEXT PRIMARY KEY,
    dropped_rows INTEGER NOT NULL DEFAULT 0
);
```

这两张表不是微信 WCDB 原生 schema，疑似此前工具/实验功能创建，当前样本均为空。备份/恢复时不要把它们当成微信消息主表。

### 5.4 WCDB 内部压缩记录

```sql
CREATE TABLE wcdb_builtin_compression_record(
    tableName TEXT PRIMARY KEY,
    columns TEXT NOT NULL,
    rowid INTEGER
) WITHOUT ROWID;
```

WCDB 内部压缩元数据，当前样本为空。

---

## 6. message/media_N.db 与 biz_media_N.db

实际表：

```sql
CREATE TABLE Name2Id(user_name TEXT PRIMARY KEY);

CREATE TABLE TimeStamp(timestamp INTEGER);

CREATE TABLE VoiceInfo(
    chat_name_id INTEGER,
    create_time INTEGER,
    local_id INTEGER,
    svr_id INTEGER,
    voice_data BLOB,
    data_index TEXT DEFAULT '0'
);
```

与旧版差异：

- `data_index` 实际是 TEXT，不是 INTEGER；
- `Name2Id` 只有 `user_name`，没有 `is_session`；
- `voice_data` 是 Silk 编码 BLOB，实际以 `#!SILK_V3` 开头，不是 WAV；
- `chat_name_id` 指向本库 `Name2Id.rowid`；
- 同一条语音可按 `svr_id + chat_name_id` 或 `local_id + create_time` 匹配。

语音处理：

```text
voice_data SilK BLOB
  -> Silk 解码 PCM
  -> 封装 WAV
```

`biz_media_0.db` 结构相同，当前样本有 2 条语音。

---

## 7. favorite/favorite.db

### 7.1 fav_db_item

实际 DDL：

```sql
CREATE TABLE fav_db_item(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id INTEGER,
    type INTEGER,
    update_seq INTEGER,
    flag INTEGER,
    update_time INTEGER,
    version INTEGER,
    content TEXT,
    source_id TEXT,
    sync_status INTEGER,
    upload_status INTEGER,
    fromusr TEXT,
    fromusr_id INTEGER,
    realchatname TEXT,
    realchatname_id INTEGER,
    ext_buf TEXT,
    upload_error_code INTEGER,
    trans_res_status INTEGER,
    trans_res_error_code INTEGER,
    star INTEGER,
    star_type INTEGER,
    star_biz_key TEXT
);
```

字段：

| 字段 | 作用 |
|---|---|
| `local_id` | 收藏本地 ID |
| `server_id` | 服务端 ID |
| `type` | 收藏类型 |
| `update_seq` | 更新序列 |
| `flag` | 标志位 |
| `update_time` | Unix 秒；实际样本为秒 |
| `version` | 版本 |
| `content` | 收藏 XML |
| `source_id` | 来源 ID |
| `sync_status` | 同步状态 |
| `upload_status` | 上传状态 |
| `fromusr` | 来源用户 |
| `fromusr_id` | 来源用户内部 ID |
| `realchatname` | 来源聊天名 |
| `realchatname_id` | 来源聊天内部 ID |
| `ext_buf` | 扩展内容，TEXT |
| `upload_error_code` | 上传错误码 |
| `trans_res_status/error_code` | 转码状态/错误 |
| `star` | 星标 |
| `star_type` | 星标类型 |
| `star_biz_key` | 星标业务键 |

实际类型分布：

```text
1 文本
2 图片
3 语音
4 视频
5 文章
6 位置
8 文件
14 聊天记录
20 视频号/finderFeed
```

本次样本还出现 `20`，其内容为 `<finderFeed>`，应归为视频号，不是普通卡片。

### 7.2 其他表

```sql
CREATE TABLE Name2Id(user_name TEXT PRIMARY KEY);

CREATE TABLE fav_tag_db_item(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id INTEGER,
    name TEXT,
    seq INTEGER
);

CREATE TABLE fav_bind_tag_db_item(
    tag_local_id INTEGER,
    tag_server_id INTEGER,
    fav_local_id INTEGER,
    fav_server_id INTEGER,
    op_code INTEGER,
    CONSTRAINT _TAGLOCALID_FAVLOCALID UNIQUE(tag_local_id, fav_local_id)
);

CREATE TABLE resync_task(
    task_id INTEGER PRIMARY KEY,
    resync_version INTEGER,
    key_buf TEXT,
    selector INTEGER,
    need_resync INTEGER
);

CREATE TABLE sync_client_storage(
    key INTEGER PRIMARY KEY,
    value TEXT,
    value_version INTEGER
);

CREATE TABLE buff(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);

CREATE TABLE config(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);
```

`buff`/`config` 是通用键值表，值可能是 int/double/text/blob 四选一。

### 7.3 favorite_fts.db

```sql
CREATE VIRTUAL TABLE fav_fts_v1
USING fts5(
    tokenize = 'MMFtsTokenizer disable_pinyin',
    content,
    local_id UNINDEXED,
    update_time UNINDEXED,
    type UNINDEXED
);
```

同样依赖 `MMFtsTokenizer`，影子表为 `_config/_content/_data/_docsize/_idx`；另有 `table_info` 键值表。

---

## 8. sns/sns.db

### 8.1 SnsTimeLine

```sql
CREATE TABLE SnsTimeLine(
    tid INTEGER PRIMARY KEY DESC,
    user_name TEXT,
    content TEXT,
    pack_info_buf TEXT
);
```

| 字段 | 作用 |
|---|---|
| `tid` | 时间线 ID，实际可能为负的大整数，使用 i64 |
| `user_name` | 发布者 |
| `content` | 朋友圈 XML |
| `pack_info_buf` | 二进制/压缩打包信息 |

### 8.2 SnsMessage_tmp3

```sql
CREATE TABLE SnsMessage_tmp3(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    create_time INTEGER,
    type INTEGER,
    feed_id INTEGER,
    is_unread INTEGER,
    from_username TEXT,
    from_nickname TEXT,
    to_username TEXT,
    to_nickname TEXT,
    content TEXT,
    serialized_comment_buf BLOB,
    serialized_ref_buf BLOB,
    comment_id INTEGER,
    client_id TEXT,
    comment64_id INTEGER,
    comment_flag INTEGER,
    del_status INTEGER,
    is_relative_me INTEGER
);
```

实际 `type` 观察到：

```text
1
2
4
```

用于区分互动/评论等；`feed_id` 关联 `SnsTimeLine.tid`，实际可为负数。

### 8.3 其他表

```sql
CREATE TABLE SnsMainTimeLineBreakFlag(
    tid INTEGER,
    tid_heigh_bit INTEGER,
    tid_low_bit INTEGER,
    break_flag INTEGER,
    CONSTRAINT _heigh_low_bit_tid PRIMARY KEY(tid_heigh_bit DESC, tid_low_bit DESC)
);

CREATE TABLE SnsUserTimeLineBreakFlagV2(
    tid INTEGER,
    tid_heigh_bit INTEGER,
    tid_low_bit INTEGER,
    break_flag INTEGER,
    user_name TEXT
);

CREATE TABLE SnsIgnoredDataItem(tid INTEGER PRIMARY KEY);

CREATE TABLE SnsErrorMessage(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    error_type INTEGER,
    creat_time INTEGER,
    tid INTEGER,
    packed_info_data TEXT
);

CREATE TABLE SnsNoteVoice(
    tid INTEGER,
    data_id TEXT,
    buff TEXT,
    CONSTRAINT _tid_data_id PRIMARY KEY(tid, data_id)
);

CREATE TABLE SnsDraft(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    create_time INTEGER,
    ui_type INTEGER,
    content TEXT,
    client_id TEXT,
    status INTEGER
);

CREATE TABLE SnsPublishTask(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    create_time INTEGER,
    ui_type INTEGER,
    content TEXT,
    client_id TEXT,
    status INTEGER
);

CREATE TABLE SnsAdTimeLine(
    tid INTEGER PRIMARY KEY DESC,
    username TEXT,
    content TEXT,
    create_time INTEGER,
    ad_content TEXT,
    ad_create_time INTEGER,
    exposure_time INTEGER,
    exposure_count INTEGER,
    remind_source_info TEXT,
    remind_self_info TEXT,
    extra_data TEXT
);

CREATE TABLE SnsTopItem_1(
    tid INTEGER,
    username TEXT,
    summary TEXT,
    create_time INTEGER,
    last_read_time INTEGER,
    is_read INTEGER
);

CREATE TABLE SnsPendingDraftDeletionTable(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    create_time INTEGER,
    ui_type INTEGER,
    content TEXT,
    client_id TEXT,
    status INTEGER,
    tid TEXT
);
```

时间字段实际为 Unix 秒；`tid/feed_id` 可能为负，必须用有符号 64 位。

---

## 9. emoticon/emoticon.db

```sql
CREATE TABLE kCustomEmoticonOrderTable(
    md5 TEXT
);

CREATE TABLE kExpressRecentUseEemoticonTable(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);

CREATE TABLE kFavEmoticonOrderTable(
    md5 TEXT
);

CREATE TABLE kNonStoreEmoticonTable(
    type INTEGER,
    md5 TEXT,
    caption TEXT,
    product_id TEXT,
    aes_key TEXT,
    thumb_url TEXT,
    tp_url TEXT,
    auth_key TEXT,
    cdn_url TEXT,
    extern_url TEXT,
    extern_md5 TEXT,
    encrypt_url TEXT,
    designer_id TEXT,
    activity_id TEXT
);

CREATE TABLE kStoreEmoticonCaptionsTable(
    package_id_ TEXT,
    md5_ TEXT,
    language_ TEXT,
    caption_ TEXT
);

CREATE TABLE kStoreEmoticonFilesTable(
    package_id_ TEXT,
    md5_ TEXT,
    type_ INTEGER,
    sort_order_ INTEGER,
    emoticon_size_ INTEGER,
    emoticon_offset_ INTEGER,
    thumb_size_ INTEGER,
    thumb_offset_ INTEGER
);

CREATE TABLE kStoreEmoticonPackageTable(
    package_id_ TEXT,
    package_name_ TEXT,
    payment_status_ INTEGER,
    download_status_ INTEGER,
    install_time_ INTEGER,
    remove_time_ INTEGER,
    sort_order_ INTEGER,
    introduction_ TEXT,
    full_description_ TEXT,
    copyright_ TEXT,
    author_ TEXT,
    store_icon_url_ TEXT,
    panel_url_ TEXT
);
```

字段重点：

- `md5` 是表情内容 MD5；
- `cdn_url`、`encrypt_url`、`extern_url` 可能是 CDN 下载地址；
- `aes_key` 是加密资源密钥；
- `package_id_` 关联表情包；
- `emoticon_size_/offset_` 表示表情包文件内偏移，用于从打包文件提取；
- `thumb_size_/offset_` 对应缩略图。

---

## 10. hardlink/hardlink.db

```sql
CREATE TABLE dir2id(
    username TEXT PRIMARY KEY
);

CREATE TABLE file_hardlink_info_v4(
    md5_hash INTEGER,
    md5 TEXT,
    type INTEGER,
    file_name TEXT,
    file_size INTEGER,
    modify_time INTEGER,
    dir1 INTEGER,
    dir2 INTEGER,
    _rowid_ INTEGER PRIMARY KEY ASC,
    extra_buffer BLOB
);

CREATE TABLE image_hardlink_info_v4(... 同上 ...);
CREATE TABLE video_hardlink_info_v4(... 同上 ...);

CREATE TABLE file_checkpoint_v4(month_id INTEGER PRIMARY KEY ASC);
CREATE TABLE video_checkpoint_v4(month_id INTEGER PRIMARY KEY ASC);

CREATE TABLE talker_checkpoint_v4(
    talker_id INTEGER PRIMARY KEY ASC,
    month_id INTEGER
);

CREATE TABLE db_info(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);
```

`dir2id` 的 rowid 是实际目录/身份索引，值中混合：

```text
月份：2024-12、2025-01 ...
会话 MD5 / 目录哈希：04ddd9ee...
```

`dir1`、`dir2` 分别指向 `dir2id.rowid`；媒体路径需要用这两个索引加 `file_name` 组合还原。

`type` 样本：

```text
1 文件
2 图片
3 视频
```

`md5_hash` 是 32 位哈希，`md5` 是完整 32 位 Hex；`_rowid_` 是主键；`extra_buffer` 是 BLOB。

---

## 11. head_image/head_image.db

```sql
CREATE TABLE head_image(
    username TEXT PRIMARY KEY,
    md5 TEXT,
    image_buffer BLOB,
    update_time INTEGER
);
```

实际 `image_buffer` 是 JPEG：

```text
ff d8 ff ...
```

`md5` 是头像内容 MD5；`update_time` 是 Unix 秒。

---

## 12. bizchat/chatbot

### 12.1 bizchat/bizchat.db

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

CREATE TABLE my_user_info(
    brand_user_name TEXT PRIMARY KEY,
    user_id TEXT
);

CREATE TABLE name2id(username TEXT PRIMARY KEY);

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

### 12.2 chatbot/chatbot_message.db

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

CREATE TABLE chatbot_session(
    username TEXT PRIMARY KEY,
    source_type INTEGER DEFAULT 0,
    open_scene INTEGER DEFAULT 0,
    timestamp INTEGER,
    tagid INTEGER DEFAULT 0,
    title TEXT,
    extra_info BLOB
);

CREATE TABLE chatbot_sort_info(
    name TEXT PRIMARY KEY,
    db_seq INTEGER DEFAULT 0,
    qa_seq INTEGER DEFAULT 0,
    timestamp INTEGER DEFAULT 0,
    is_end INTEGER DEFAULT 0
);
```

---

## 13. general/general.db

实际 24 张表：

```text
FMessageTable
ForwardRecent
SearchRecent
WeAppBizAttrSyncBufferTableV02
biz_pay_status
biz_subscribe_status
brand_search_record
groupPayTable
handoff_remind_v0
ilink_voip
new_tips
redEnvelopeTable
reddot
reddot_last_notify
reddot_record
revokebatchmessage
revokemessage
sqlite_sequence
teenager_apply_access_agree_info
transferTable
wacontact
wcfinderlivestatus
wcfinderuserpage
websearch_record
```

关键 DDL：

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

CREATE TABLE ForwardRecent(
    username TEXT,
    forward_time INTEGER
);

CREATE TABLE SearchRecent(
    username TEXT,
    query TEXT,
    score INTEGER,
    last_click_time INTEGER,
    CONSTRAINT _query_username UNIQUE(query, username)
);

CREATE TABLE WeAppBizAttrSyncBufferTableV02(
    user_name TEXT PRIMARY KEY,
    last_update_time INTEGER,
    version TEXT
);

CREATE TABLE biz_pay_status(
    url_id TEXT,
    is_charge_appmsg INTEGER,
    is_paid INTEGER,
    friend_pay_count_str TEXT
);

CREATE TABLE biz_subscribe_status(
    biz_username TEXT,
    template_id TEXT,
    is_subscribe INTEGER
);

CREATE TABLE brand_search_record(
    keyword TEXT PRIMARY KEY,
    pay_load_ TEXT,
    create_time INTEGER
);

CREATE TABLE groupPayTable(
    bill_no TEXT PRIMARY KEY,
    session_name TEXT,
    message_local_id INTEGER,
    message_create_time INTEGER
);

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

CREATE TABLE new_tips(
    unique_id TEXT PRIMARY KEY,
    disable INTEGER,
    new_tips_content TEXT
);

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

CREATE TABLE revokebatchmessage(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER,
    msg_unique_id TEXT,
    session_name TEXT,
    msg_local_id INTEGER,
    msg_create_time INTEGER
);

CREATE TABLE revokemessage(
    to_user_name TEXT,
    svr_id INTEGER,
    message_type INTEGER,
    revoke_time INTEGER,
    content TEXT,
    at_user_list TEXT
);

CREATE TABLE teenager_apply_access_agree_info(
    access_content_key TEXT PRIMARY KEY,
    access_content_type INTEGER,
    agree_time INTEGER
);

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

CREATE TABLE wcfinderlivestatus(
    finder_live_id INTEGER,
    finder_username TEXT,
    finder_export_id TEXT,
    live_status INTEGER,
    replay_status INTEGER,
    charge_flag INTEGER
);

CREATE TABLE wcfinderuserpage(
    username TEXT,
    extra_buffer BLOB
);

CREATE TABLE websearch_record(
    keyword TEXT PRIMARY KEY,
    pay_load_ TEXT,
    create_time INTEGER
);
```

字段说明：

- `FMessageTable`：好友请求/系统通知；
- `ForwardRecent`：最近转发；
- `SearchRecent`：最近搜索；
- `transferTable`：转账记录；
- `redEnvelopeTable`：红包记录；
- `revokemessage`/`revokebatchmessage`：撤回记录；
- `wacontact`：公众号/小程序联系人；
- `wcfinder*`：视频号相关状态；
- `groupPayTable`：群收款；
- `handoff_remind_v0`：接力提醒。

---

## 14. message/message_resource.db

```sql
CREATE TABLE ChatName2Id(
    user_name TEXT PRIMARY KEY,
    update_time INTEGER
);

CREATE TABLE SenderName2Id(
    user_name TEXT PRIMARY KEY,
    update_time INTEGER
);

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

关系：

```text
MessageResourceInfo.message_id
  -> MessageResourceDetail.message_id
```

`chat_id`/`sender_id` 是内部映射 ID，分别对应 `ChatName2Id`/`SenderName2Id` 的 rowid（具体映射需按实际库验证）。

`packed_info` 是 protobuf 媒体信息；`MessageResourceDetail.type` 是资源类型，`size`/`create_time`/`access_time`/`status` 是资源状态。

---

## 15. solitaire 与 third_app_icon

### 15.1 solitaire/solitaire.db

每个群/会话一张前缀表：

```text
Solitaire_<hash>
SolitaireValid_<hash>
SolitaireFold_<hash>
```

DDL：

```sql
CREATE TABLE Solitaire_<hash>(
    header TEXT,
    footer TEXT,
    content TEXT,
    solitaire_string TEXT,
    active_time INTEGER,
    CONSTRAINT SOLITAIRE_UNIQUE_INDEX UNIQUE(header, footer)
);

CREATE TABLE SolitaireValid_<hash>(
    title TEXT,
    valid INTEGER,
    local_id INTEGER,
    sort_seq INTEGER,
    create_time INTEGER,
    CONSTRAINT SOLITAIRE_VALID_UNIQUE_INDEX UNIQUE(local_id, sort_seq)
);

CREATE TABLE SolitaireFold_<hash>(
    front_content TEXT,
    behind_content TEXT,
    local_id INTEGER,
    sort_seq INTEGER,
    create_time INTEGER,
    CONSTRAINT SOLITAIRE_FOLD_UNIQUE_INDEX UNIQUE(local_id, sort_seq)
);
```

`<hash>` 是会话/群相关哈希；`Solitaire` 是主接龙，`Valid` 是有效条目，`Fold` 是折叠内容。

### 15.2 third_app_icon/third_app_icon.db

```sql
CREATE TABLE third_app_icon(
    app_key TEXT PRIMARY KEY,
    icon_url_md5 TEXT,
    image_buffer BLOB,
    update_time INTEGER
);
```

`image_buffer` 是图标二进制；`icon_url_md5` 是 URL/内容 MD5。

---

## 16. message/message_fts.db

实际 FTS 虚表：

```sql
CREATE VIRTUAL TABLE message_fts_v4_0
USING fts5(
    tokenize = 'MMFtsTokenizer disable_pinyin',
    acontent,
    message_local_id UNINDEXED,
    sort_seq UNINDEXED,
    local_type UNINDEXED,
    session_id UNINDEXED,
    sender_id UNINDEXED,
    create_time UNINDEXED
);
```

类似还有 `message_fts_v4_1`。

辅助表：

```sql
CREATE TABLE message_fts_v4_aux_0(
    message_local_id INTEGER,
    sort_seq INTEGER,
    session_id INTEGER,
    CONSTRAINT sessionId_localId_sortseq PRIMARY KEY(session_id, message_local_id, sort_seq)
);

CREATE TABLE message_fts_v4_range(
    db_time_stamp INTEGER,
    start_local_id INTEGER,
    end_local_id INTEGER,
    session_id INTEGER,
    CONSTRAINT sessionId_dbTime PRIMARY KEY(session_id, db_time_stamp)
);

CREATE TABLE message_fts_v4_session_delete_info(
    session_id INTEGER,
    start_local_id INTEGER,
    end_local_id INTEGER,
    db_time_stamp INTEGER,
    CONSTRAINT sessionId_dbtime PRIMARY KEY(session_id, db_time_stamp)
);

CREATE TABLE ImgFts0V0 ... ImgFts3V0
CREATE TABLE ImgFtsAux0V0 ... ImgFtsAux3V0
CREATE TABLE ImgRangeV0
CREATE TABLE deleteImgFtsV0
CREATE TABLE ImgTableInfo
CREATE TABLE name2id
CREATE TABLE table_info
```

注意：

- `MMFtsTokenizer` 是自定义 tokenizer，普通 SQLite 不能直接查询虚表；
- 影子表 `*_config/_content/_data/_docsize/_idx` 必须与虚表一起备份；
- 只恢复消息主表而不同步 FTS，全文搜索会看不到新恢复的消息；
- 可以通过重建 FTS 索引修复，但需要知道原 tokenizer 和 schema。

---

## 17. 消息类型与字段处理

### 17.1 基础类型

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
| 49 | appmsg |
| 50 | 通话 |
| 10000/10002 | 系统消息 |

### 17.2 appmsg 子类型

| sub_type | 含义 |
|---:|---|
| 3 | 音乐 |
| 5/49 | 链接/公众号文章 |
| 6 | 文件 |
| 8 | 视频/其它 app |
| 19 | 聊天记录 |
| 33/36 | 小程序 |
| 51/63/115 | 视频号 |
| 57 | 引用 |
| 62 | 拍一拍 |
| 87 | 群公告 |
| 2000 | 转账 |
| 2001 等 | 扩展业务 |

### 17.3 关键处理

1. **zstd**：`WCDB_CT_* == 4` 或 `28 B5 2F FD`；
2. **BLOB/TEXT 动态类型**：先取字节，不解码；
3. **`compress_content` 回退**：主内容空时使用；
4. **`local_type` 64 位拆分**：不要只看低 32 位；
5. **`sort_seq` 13 位**：按时间/顺序键处理，不要当小序号；
6. **`server_id` 64 位**：使用 i64/u64；
7. **`real_sender_id`**：仅本库 `Name2Id` 有效；
8. **群发送者前缀**：`wxid:\n` 优先；
9. **`<patMsg>` 不是拍一拍**：只有 `<pat>`、`<sysmsg type="pat">`、子类型 62 才是；
10. **`<sysmsg>` 顶层判断**：不要因嵌套文本误判系统消息；
11. **XML 转义**：CDATA/实体/嵌套 XML；
12. **语音**：Silk BLOB，不是 WAV；
13. **媒体**：MD5/URL 需结合 `hardlink`、`message_resource` 和本地媒体目录；
14. **FTS**：虚表 + 影子表整体处理；
15. **非原生表**：`alpha_message_recall_*` 不是微信 WCDB 原生 schema。

---

## 18. 备份与恢复注意

- 完整 DB 备份应复制 `.db` + `-wal` + `-shm`；
- 只恢复消息表而不恢复 FTS，搜索会缺失；
- 只恢复消息主表而不恢复 `Name2Id`，发送者映射会错；
- 只恢复消息表而不恢复 `SessionTable`，会话列表可能不显示；
- 媒体文件需要 `hardlink`/`message_resource` 与本地文件一起处理；
- 恢复前先备份目标 DB；
- 表结构可能随版本变化，必须动态查询 `sqlite_master`/`PRAGMA table_info`。

---

## 19. 实际采样中的特殊现象

- `session.SessionTable.type` 全为 0，不能用于区分私聊/群聊；
- `contact.contact.chat_room_notify` 实际为 INTEGER；
- `message.Name2Id` 实际为 `is_session`，不是 `is_group`；
- `VoiceInfo.data_index` 实际为 TEXT `'0'`；
- `WCDB_CT_source` 大量为 4，`source` 为 zstd BLOB；
- `compress_content` 本次全部为空，但代码应保留回退；
- `sort_seq` 实际为 13 位；
- `SnsTimeLine.tid`/`feed_id` 可为负，必须用 i64；
- `favorite` 存在 type=20 的视频号 `finderFeed`；
- `head_image.image_buffer` 为 JPEG；
- `VoiceInfo.voice_data` 以 `#!SILK_V3` 开头；
- `hardlink.dir2id` 同时存月份和会话/目录哈希；
- `contact_fts`/`message_fts`/`favorite_fts` 使用 `MMFtsTokenizer`，普通 SQLite 无法直接查询虚表；
- `alpha_message_recall_*` 非微信原生。
