# 微信 4.x WCDB 分册文档

这是一套面向零基础的微信 WCDB 数据库说明。每一册只讲一个数据库或一组强相关数据库，包含：

- 这个库是做什么的；
- 实际表结构；
- 每个字段是什么意思；
- 哪些字段需要 zstd、XML、BLOB 等特殊处理；
- 可以直接运行的 Python 示例；
- 实际采样中发现的差异和坑。

## 先看这里

如果你还没解密微信数据库，请先看总文档的“新手先读”：

- [WCDB 总览与零基础教程](../WCDB.md)

准备环境：

```bash
python --version
pip install zstandard
```

解密后的目录通常叫 `db_decrypted`：

```text
db_decrypted/
├── session/session.db
├── contact/contact.db
├── message/message_0.db
├── message/message_1.db
├── favorite/favorite.db
└── ...
```

下面所有 Python 示例都假设你已经有这个目录，并且使用**只读方式**打开数据库。

## 分册目录

| 文档 | 数据库 | 内容 |
|---|---|---|
| [01-session.md](01-session.md) | `session/session.db` | 会话列表、未读、草稿、会话删除 |
| [02-contact.md](02-contact.md) | `contact/contact.db` | 联系人、好友、群、群成员、标签、公众号、企业微信 |
| [03-message.md](03-message.md) | `message/message_N.db`、`biz_message_N.db` | 消息主表、Name2Id、消息辅助表、恢复相关表 |
| [04-media.md](04-media.md) | `message/media_N.db`、`biz_media_N.db` | 语音、Silk、媒体索引 |
| [05-favorite.md](05-favorite.md) | `favorite/favorite.db` | 微信收藏、类型、XML、时间戳 |
| [06-sns.md](06-sns.md) | `sns/sns.db` | 朋友圈、点赞、评论、置顶、草稿、广告 |
| [07-emoticon.md](07-emoticon.md) | `emoticon/emoticon.db` | 表情包、CDN、文件偏移、收藏顺序 |
| [08-hardlink.md](08-hardlink.md) | `hardlink/hardlink.db` | 图片/视频/文件 MD5 到真实路径 |
| [09-head-image.md](09-head-image.md) | `head_image/head_image.db` | 头像缓存 |
| [10-bizchat-chatbot.md](10-bizchat-chatbot.md) | `bizchat`、`chatbot` | 企业微信/客服、聊天机器人 |
| [11-general.md](11-general.md) | `general/general.db` | 转账、红包、撤回、搜索、视频号状态等 |
| [12-message-resource.md](12-message-resource.md) | `message/message_resource.db` | 消息资源、媒体资源详情、FTS 范围 |
| [13-solitaire-third-app-icon.md](13-solitaire-third-app-icon.md) | `solitaire`、`third_app_icon` | 群接龙、第三方应用图标 |
| [14-fts.md](14-fts.md) | `contact_fts`、`favorite_fts`、`message_fts` | 微信 FTS、MMFtsTokenizer、影子表 |
| [15-internal-tables.md](15-internal-tables.md) | 各消息库的内部表 | 删除、历史、时间戳、非微信原生 `alpha_*` |

## 三个最重要的概念

### 1. 一个文件就是一个 SQLite 数据库

```python
import sqlite3

conn = sqlite3.connect(
    "file:session/session.db?mode=ro",
    uri=True,
)
print(conn.execute("SELECT count(*) FROM SessionTable").fetchone())
```

- `mode=ro` 表示只读；
- 不要用普通 `connect("xxx.db")` 修改原始数据；
- 微信原始库是加密的，必须先用 `wechat-exporter decrypt`。

### 2. 表名经常是 hash

消息表不是固定名字，而是：

```text
Msg_<md5(会话ID)>
```

例如：

```python
import hashlib

conversation_id = "123456789@chatroom"
table = "Msg_" + hashlib.md5(conversation_id.encode()).hexdigest()
```

### 3. 字段可能被压缩

看到 `message_content`、`source`、`content` 时：

1. 先看它是不是 BLOB；
2. 看 `WCDB_CT_*` 是否为 `4`；
3. 看开头是不是 zstd 魔数 `28 B5 2F FD`；
4. 解压后再 UTF-8 解码；
5. 去末尾 NUL；
6. 如果是 XML 再解析 XML。

```python
ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"
```

## 常见错误

| 错误 | 原因 | 解决 |
|---|---|---|
| `file is not a database` | 打开了未加密库 | 先用 `wechat-exporter decrypt` |
| `no such table` | 打开错文件/表名错 | 用分册确认路径和表名 |
| `no such tokenizer: MMFtsTokenizer` | 微信自定义 FTS | 看 [14-fts.md](14-fts.md) |
| 内容是乱码 | 直接 UTF-8 解码 zstd | 先解压 |
| 消息是 XML | 正常 | 解析 XML 或看分册代码 |
| 找不到某会话消息 | 表名 MD5 不对 | 用会话 ID 计算 MD5 |
| 语音打不开 | Silk 不是 WAV | 看 [04-media.md](04-media.md) |

## 总原则

- 先备份，再操作；
- 分析时只读打开；
- 不要跨数据库复用 `Name2Id.rowid`；
- 不要假设 `sort_seq` 是小序号；
- 不要用 `<patMsg>` 判断拍一拍；
- 不要把 FTS 虚表当普通表；
- 不要把 `alpha_message_recall_*` 当微信原生表。
