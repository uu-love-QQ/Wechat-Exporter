# 微信 4.x WCDB 数据库与表结构摘要

> 适用范围：Windows / Linux 上的微信 4.x WCDB（SQLCipher 4）。
> 表结构会随微信版本和账号状态变化；本文把“已实际观察”的结构与“尚未采样”的数据库分开标注。
> 生产代码应通过 `sqlite_master`、`PRAGMA table_info` 动态探测，而不是假设每个账号完全一致。

## 1. 实际 db_storage 文件清单

一个真实微信 4.x 账号的 `db_storage` 常见如下：

```text
db_storage/
├── bizchat/bizchat.db
├── chatbot/chatbot_message.db
├── contact/contact.db
├── contact/contact_fts.db
├── emoticon/emoticon.db
├── favorite/favorite.db
├── favorite/favorite_fts.db
├── general/general.db
├── hardlink/hardlink.db
├── head_image/head_image.db
├── message/
│   ├── biz_media_0.db
│   ├── biz_message_0.db
│   ├── biz_message_1.db
│   ├── biz_message_2.db
│   ├── media_0.db
│   ├── media_1.db
│   ├── message_0.db
│   ├── message_1.db
│   ├── message_2.db
│   ├── message_3.db
│   ├── message_fts.db
│   ├── message_resource.db
│   └── weclaw.db
├── session/session.db
├── sns/sns.db
├── solitaire/solitaire.db
└── third_app_icon/third_app_icon.db
```

说明：

| 文件 | 主要用途 | 当前采样程度 |
|---|---|---|
| `session/session.db` | 会话列表、未读、最后一条消息 | 已实际观察 |
| `contact/contact.db` | 联系人、群成员、标签、公众号资料 | 已实际观察 |
| `message/message_N.db` | 私聊/群聊消息分片 | 已实际观察 |
| `message/biz_message_N.db` | 公众号、服务号等业务消息分片 | 结构与 message 基本一致 |
| `message/media_N.db` | 语音、图片、视频、文件等媒体索引 | 已观察 VoiceInfo 等表 |
| `message/biz_media_N.db` | 业务消息媒体索引 | 结构接近 media |
| `message/message_fts.db` | 消息全文检索 FTS 索引 | 需按 FTS 处理 |
| `message/message_resource.db` | 消息资源/媒体映射 | 未完整采样 |
| `favorite/favorite.db` | 微信收藏 | 已观察 fav_db_item |
| `favorite/favorite_fts.db` | 收藏全文索引 | FTS |
| `contact/contact_fts.db` | 联系人全文索引 | FTS |
| `sns/sns.db` | 朋友圈、评论、点赞、草稿 | 已实际观察 |
| `emoticon/emoticon.db` | 表情包元数据 | 版本差异大 |
| `hardlink/hardlink.db` | 媒体 MD5 -> 本地真实路径 | 部分观察 |
| `head_image/head_image.db` | 头像缓存 | 表名/字段版本差异大 |
| `general/general.db` | 通用设置/状态 | 未完整采样 |
| `bizchat/bizchat.db` | 企业微信/客服会话扩展 | 未完整采样 |
| `chatbot/chatbot_message.db` | 聊天机器人消息 | 未完整采样 |
| `solitaire/solitaire.db` | 群接龙 | 未完整采样 |
| `third_app_icon/third_app_icon.db` | 第三方应用图标 | 未完整采样 |
| `message/weclaw.db` | WeClaw/小程序扩展数据 | 未完整采样 |

---

## 2. 通用规则

### 2.1 SQLCipher 4

- AES-256-CBC + HMAC-SHA512；
- 页大小通常 4096；
- 每页尾部 80 字节保留区：16 字节 IV + 64 字节 HMAC；
- 每个数据库文件前 16 字节是 salt；
- 用户传入的 64 位 Hex 通常是主通行码，不是单库最终 key；
- 最终 key 通常由 `PBKDF2-HMAC-SHA512(master, per-db salt, 256000)` 派生；
- 同一主通行码在不同库上得到不同 key；
- 也存在直接传入单库 raw key 的兼容路径。

### 2.2 WAL

数据库旁边可能有：

```text
xxx.db
xxx.db-wal
xxx.db-shm
```

读取、备份或恢复时必须把三者视为一个整体，否则最近的会话、消息、互动会丢失。

### 2.3 zstd / BLOB

消息表常见：

```text
WCDB_CT_message_content
WCDB_CT_source
```

处理规则：

- `WCDB_CT_* == 4` 时按 zstd 处理；
- 同时检查 zstd 魔数 `28 B5 2F FD`；
- `message_content` 为空时回退到 `compress_content`；
- `source` 也可能压缩，使用 `WCDB_CT_source`；
- 压缩字段必须是 BLOB/字节，不能先按 UTF-8 字符串读取；
- 解压后去掉末尾 NUL（`\0`）；
- `packed_info_data`、`pack_info_buf`、`serialized_*_buf` 是二进制，可能是 protobuf 或 zstd，不能当普通文本。

### 2.4 `local_type` 打包

```text
base_type = local_type & 0xFFFFFFFF
sub_type  = local_type >> 32
```

常见基础类型：

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
| 49 | appmsg 应用消息 |
| 50 | 通话 |
| 10000 / 10002 | 系统消息 |

`base_type=49` 的子类型：

| sub_type | 含义 |
|---:|---|
| 3 | 音乐 |
| 5 / 49 | 链接/文章 |
| 6 | 文件 |
| 19 | 聊天记录 |
| 33 / 36 | 小程序 |
| 51 / 63 / 115 | 视频号 |
| 57 | 引用 |
| 87 | 群公告 |
| 2000 | 转账 |
| 62 | 拍一拍 |

示例：

```text
244813135921 = (57 << 32) | 49   -> 引用消息
266287972401 = (62 << 32) | 49   -> 拍一拍
```

### 2.5 发送者

- 消息表 `real_sender_id` 指向 **本数据库** 的 `Name2Id.rowid`；
- `Name2Id` 每个消息分片独立，不能跨库复用 rowid；
- 群文本可能带 `wxid:\n` 前缀，前缀通常比历史 `real_sender_id` 更可靠；
- 群消息的发送者也可能是 `system` 或群成员。

### 2.6 XML

`message_content`、`source`、收藏内容、朋友圈内容常见：

- XML 声明；
- CDATA；
- 转义后的嵌套 XML：`&lt;appmsg&gt;`；
- HTML/XML 实体：`&amp;`、`&#x20;`；
- 引用 `<refermsg>`；
- 应用 `<appmsg>`；
- 系统 `<sysmsg>`；
- 拍一拍 `<sysmsg type="pat">` / `<pat>`。

注意：很多普通 appmsg 都带空 `<patMsg>` 元数据；`<patMsg>` 不是拍一拍，不能据此判断。

---

## 3. session/session.db

主表：

```sql
CREATE TABLE SessionTable (
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

| 字段 | 作用 |
|---|---|
| `username` | 会话 ID，`wxid_*` / `*@chatroom` / `gh_*` |
| `type` | 会话类型 |
| `unread_count` | 未读数 |
| `unread_first_msg_srv_id` | 未读起点 server_id |
| `unread_first_pat_msg_local_id` | 未读拍一拍 local_id |
| `unread_first_pat_msg_sort_seq` | 未读拍一拍排序号 |
| `is_hidden` | 是否隐藏 |
| `summary` | 会话摘要 |
| `draft` | 草稿 |
| `last_timestamp` | 最后消息时间，通常 Unix 秒 |
| `sort_timestamp` | 会话排序时间，通常 Unix 秒 |
| `last_clear_unread_timestamp` | 清未读时间 |
| `last_msg_locald_id` | 最后消息 local_id |
| `last_msg_type` | 最后消息基础类型 |
| `last_msg_sub_type` | 最后消息子类型 |
| `last_msg_sender` | 最后消息发送者 |
| `last_sender_display_name` | 最后发送者显示名 |
| `last_msg_ext_type` | 扩展类型 |

---

## 4. contact/contact.db

### 4.1 contact

```sql
CREATE TABLE contact (
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
    chat_room_notify TEXT,
    is_in_chat_room INTEGER,
    description TEXT,
    extra_buffer BLOB,
    chat_room_type INTEGER
);
```

| 字段 | 作用 / 注意 |
|---|---|
| `username` | wxid、群、公众号、企业 ID |
| `local_type` | 联系人类型位掩码 |
| `alias` | 微信号 |
| `encrypt_username` | 加密 ID/别名 |
| `flag` | 联系人标志位 |
| `delete_flag` | 删除标志 |
| `verify_flag` | 验证状态 |
| `remark` | 备注名 |
| `remark_quan_pin` | 备注全拼 |
| `remark_pin_yin_initial` | 备注拼音首字母 |
| `nick_name` | 昵称 |
| `pin_yin_initial` | 昵称拼音首字母 |
| `quan_pin` | 昵称全拼 |
| `big_head_url` / `small_head_url` | 头像 URL |
| `head_img_md5` | 头像 MD5 |
| `chat_room_notify` | 群公告/通知 |
| `is_in_chat_room` | 是否仍在群内 |
| `description` | 描述 |
| `extra_buffer` | 二进制扩展，常见 protobuf |
| `chat_room_type` | 群类型 |

辅助表：

```sql
CREATE TABLE chatroom_member (
    room_id INTEGER,
    member_id INTEGER
);

CREATE TABLE name2id (
    user_name TEXT
);
```

`room_id/member_id` 通常是本库内部 id 或 `contact.id`，不能直接当 wxid。

部分版本还有：

```text
contact_label      联系人标签
biz_info           公众号/服务号资料
```

### 4.2 contact_fts.db

FTS 全文索引，通常包含：

```text
contact_fts
contact_fts_data
contact_fts_idx
contact_fts_content
contact_fts_docsize
contact_fts_config
```

这些是 FTS5 虚表/影子表，不能只复制 `contact_fts` 而丢掉影子表。

---

## 5. message/message_N.db

### 5.1 Name2Id

```sql
CREATE TABLE Name2Id (
    user_name TEXT,
    is_group INTEGER
);
```

`rowid` 是消息表 `real_sender_id` 的映射目标；`is_group=1` 表示群 ID。

### 5.2 Msg_<md5(username)>

```sql
CREATE TABLE Msg_<md5(username)> (
    local_id INTEGER PRIMARY KEY,
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
    message_content BLOB,
    compress_content BLOB,
    packed_info_data BLOB,
    WCDB_CT_message_content INTEGER,
    WCDB_CT_source INTEGER
);
```

表名：

```text
Msg_<md5(会话 username)>
```

| 字段 | 作用 |
|---|---|
| `local_id` | 会话内本地消息 ID |
| `server_id` | 服务端消息 ID，可能是 64 位 |
| `local_type` | 64 位打包类型 |
| `sort_seq` | 同秒消息排序 |
| `real_sender_id` | 指向本库 `Name2Id.rowid` |
| `create_time` | Unix 秒 |
| `status` | 消息状态 |
| `upload_status` | 上传状态 |
| `download_status` | 下载状态 |
| `server_seq` | 服务端顺序 |
| `origin_source` | 来源标志 |
| `source` | 来源 XML，可能 zstd |
| `message_content` | 主内容，TEXT/BLOB/zstd |
| `compress_content` | 备用压缩内容 |
| `packed_info_data` | 二进制媒体/扩展信息 |
| `WCDB_CT_message_content` | 内容编码标记，`4` 常见为 zstd |
| `WCDB_CT_source` | source 编码标记 |

`biz_message_N.db` 使用同样的 `Msg_` 结构，主要用于公众号/服务号。

### 5.3 VoiceInfo

```sql
CREATE TABLE VoiceInfo (
    chat_name_id INTEGER,
    create_time INTEGER,
    local_id INTEGER,
    svr_id INTEGER,
    voice_data BLOB,
    data_index INTEGER
);
```

| 字段 | 作用 |
|---|---|
| `chat_name_id` | 指向本库 `Name2Id.rowid` |
| `create_time` | Unix 秒 |
| `local_id` | 对应消息 local_id |
| `svr_id` | 对应消息 server_id |
| `voice_data` | **Silk 编码 BLOB，不是 WAV** |
| `data_index` | 同秒语音排序 |

版本差异可能出现：

```text
voice_data / buf / voicebuf / data
svr_id / msg_svr_id / svrid
chat_name_id / chatnameid
```

语音处理流程：

1. 按 `svr_id + chat_name_id` 或 `local_id + create_time` 匹配；
2. 读取 Silk BLOB；
3. Silk 解码为 PCM；
4. PCM 封装 WAV。

---

## 6. message/media_N.db 与 biz_media_N.db

主要保存媒体资源索引：

```text
Name2Id
VoiceInfo
图片/视频/文件/表情相关表
```

`VoiceInfo` 同上。其余表随版本差异较大，建议按表名和列名动态探测。

---

## 7. favorite/favorite.db

核心表：

```text
fav_db_item
```

本仓库实际使用字段：

```text
local_id / localId / id
type / Type
update_time / updateTime / create_time / createTime
content / Content
fromusr / from_username / fromUser
realchatname / chat_name / chatName
```

| 字段 | 作用 |
|---|---|
| `local_id` | 收藏本地 ID |
| `type` | 收藏类型 |
| `update_time/create_time` | 收藏时间，可能秒或毫秒 |
| `content` | 收藏 XML |
| `fromusr` | 来源用户 |
| `realchatname` | 来源聊天 |

常见类型：

```text
1 文本, 2 图片, 3 语音, 4 视频, 5 文章,
6 位置, 8 文件, 14 聊天记录, 16 笔记,
17 小程序, 19 卡片
```

特殊处理：

- 时间戳大于 `9999999999` 通常按毫秒处理；
- `content` 是 XML，可能包含 CDATA、转义嵌套 XML；
- 语音、视频、文件通常只有 URL/MD5/时长，媒体二进制不在 DB 内。

---

## 8. sns/sns.db

实际观察到的表：

```text
SnsTimeLine
SnsMainTimeLineBreakFlag
SnsUserTimeLineBreakFlagV2
SnsMessage_tmp3
SnsIgnoredDataItem
SnsErrorMessage
SnsNoteVoice
SnsDraft
SnsPublishTask
SnsAdTimeLine
SnsTopItem_1
SnsPendingDraftDeletionTable
```

### 8.1 SnsTimeLine

```sql
CREATE TABLE SnsTimeLine (
    tid INTEGER PRIMARY KEY DESC,
    user_name TEXT,
    content TEXT,
    pack_info_buf TEXT
);
```

| 字段 | 作用 |
|---|---|
| `tid` | 朋友圈时间线 ID |
| `user_name` | 发布者 |
| `content` | 朋友圈 XML |
| `pack_info_buf` | 二进制/压缩打包信息 |

`content` 内常见：

```text
<TimelineObject>
  <id>...</id>
  <username>...</username>
  <createTime>...</createTime>
  <contentDesc>...</contentDesc>
  <objectDesc>...</objectDesc>
  <likeUserList>...</likeUserList>
  <commentList>...</commentList>
</TimelineObject>
```

### 8.2 SnsMessage_tmp3

```sql
CREATE TABLE SnsMessage_tmp3 (
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

| 字段 | 作用 |
|---|---|
| `feed_id` | 关联 `SnsTimeLine.tid` |
| `type` | 互动类型 |
| `from_username` | 发起人 |
| `to_username` | 被互动人 |
| `content` | 评论内容 |
| `serialized_comment_buf` | 评论序列化数据，可能 protobuf/zstd |
| `serialized_ref_buf` | 引用序列化数据 |
| `comment_id/comment64_id` | 评论 ID，注意位宽 |
| `del_status` | 删除状态 |
| `is_relative_me` | 是否与我相关 |

其他表：

- `SnsDraft`：草稿；
- `SnsPublishTask`：发布任务；
- `SnsAdTimeLine`：广告时间线；
- `SnsNoteVoice`：语音笔记；
- `SnsTopItem_1`：置顶动态；
- `SnsIgnoredDataItem`：忽略项；
- `SnsErrorMessage`：错误记录；
- `SnsMainTimeLineBreakFlag` / `SnsUserTimeLineBreakFlagV2`：时间线断点/分页辅助。

时间字段通常为 Unix 秒；部分发布/互动时间可能为毫秒，按数量级判断。

---

## 9. emoticon / hardlink / head_image

### 9.1 emoticon/emoticon.db

表情包元数据库。表名随版本变化，常见字段会包含：

```text
md5 / cdnurl / encrypturl / name / package_id / type
```

处理建议：

1. 先列出 `sqlite_master`；
2. 找包含 `md5`、`cdnurl`、`name` 的表；
3. 多候选表按列名动态匹配，不要硬编码固定表。

### 9.2 hardlink/hardlink.db

媒体 MD5 到本地真实文件路径的映射。已知常见表：

```text
video_hardlink_info_v4
```

图片、视频、文件可能各自使用不同 hardlink 表；列名和表名版本差异大，应动态查找 `md5`、`file_name`、`dir`、`size` 等字段。

常见路径：

```text
db_storage/hardlink/hardlink.db
db_storage/hardlink.db
账号根目录/hardlink.db
```

### 9.3 head_image/head_image.db

头像缓存库。常见用途：

- `username` / `user_name` -> 头像 BLOB；
- 头像 MD5、更新时间；
- 部分版本通过微信 native API 读取，不暴露固定表。

不要假设固定表名；按列名动态探测。

---

## 10. FTS 数据库

以下文件通常是 FTS 索引：

```text
contact/contact_fts.db
favorite/favorite_fts.db
message/message_fts.db
```

FTS5 会生成影子表：

```text
*_data
*_idx
*_content
*_docsize
*_config
```

备份/恢复要求：

- 必须把 FTS 虚表和影子表一起处理；
- 不要只复制主虚表；
- 如果只恢复了消息表而没有恢复 FTS，搜索会缺少新恢复的消息；
- 可以通过重新建索引解决，但需要知道原 FTS schema。

---

## 11. 其余数据库

以下文件已在实际账号中出现，但当前仓库尚未完整逆向其表结构，使用前必须动态探测：

| 文件 | 用途推测/已知 |
|---|---|
| `general/general.db` | 通用设置、全局状态 |
| `bizchat/bizchat.db` | 企业微信/客服会话扩展 |
| `chatbot/chatbot_message.db` | 聊天机器人消息 |
| `message/message_resource.db` | 消息资源/媒体映射 |
| `message/weclaw.db` | WeClaw/小程序扩展数据 |
| `solitaire/solitaire.db` | 群接龙 |
| `third_app_icon/third_app_icon.db` | 第三方应用图标 |

对这些库应：

1. `SELECT name, type, sql FROM sqlite_master`；
2. `PRAGMA table_info(<table>)`；
3. 根据实际列名建立读取逻辑；
4. 不要按其他账号/版本的固定 schema 假设字段存在。

---

## 12. 解析注意事项清单

1. 先判断 BLOB/压缩，再做 UTF-8 解码。
2. `WCDB_CT_* == 4` 或 zstd 魔数 `28 B5 2F FD` 时解压。
3. `message_content` 为空时回退 `compress_content`。
4. 去掉末尾 NUL。
5. `local_type` 必须拆低 32 位基础类型和高 32 位子类型。
6. `base_type=49` 必须解析 XML `<type>`。
7. `<patMsg>` 不是拍一拍；只有 `<pat>` / `type="pat"` / 子类型 62 才是。
8. `<sysmsg>` 只应看顶层结构，不能因为嵌套文本误判系统消息。
9. 群消息优先使用文本里的 `wxid:\n` 发送者前缀。
10. `real_sender_id` 只能在本库 `Name2Id` 中解析。
11. `server_id` 可能是 64 位无符号整数，使用 `i64`/`u64`。
12. `create_time` 多数是秒；收藏/朋友圈/部分资源可能是毫秒。
13. 收藏 XML、朋友圈 XML、消息 XML 都可能包含转义嵌套 XML。
14. 语音 BLOB 是 Silk，不是 WAV。
15. 媒体字段只记录 MD5/URL 时，必须结合 hardlink/media 索引和本地媒体目录。
16. FTS 虚表和影子表必须整体处理。
17. 表结构随版本变化，始终动态探测。
