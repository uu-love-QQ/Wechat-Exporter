# favorite/favorite.db：微信收藏

## 1. 这个库是做什么的

`favorite/favorite.db` 保存微信“收藏”：

- 文本；
- 图片；
- 语音；
- 视频；
- 文章；
- 文件；
- 聊天记录；
- 位置；
- 笔记；
- 小程序；
- 名片/卡片；
- 视频号内容。

收藏和聊天记录是不同的东西：收藏不依赖某个会话存在。

## 2. 实际表清单

本次样本 9 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `fav_db_item` | 74 | 收藏主表 |
| `fav_tag_db_item` | 0 | 收藏标签 |
| `fav_bind_tag_db_item` | 0 | 收藏与标签绑定 |
| `Name2Id` | 204 | 内部 ID 映射 |
| `resync_task` | 1 | 重新同步任务 |
| `sync_client_storage` | 1 | 同步状态 |
| `buff` | 0 | 键值缓冲 |
| `config` | 2 | 配置 |
| `sqlite_sequence` | 1 | 自增序列 |

## 3. 收藏主表 fav_db_item

### 3.1 实际 DDL

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

### 3.2 字段表

| 字段 | 类型 | 含义 |
|---|---|---|
| `local_id` | INTEGER | 收藏本地 ID |
| `server_id` | INTEGER | 服务端 ID |
| `type` | INTEGER | 收藏类型，见下表 |
| `update_seq` | INTEGER | 更新序列 |
| `flag` | INTEGER | 标志位 |
| `update_time` | INTEGER | 更新时间，Unix 秒 |
| `version` | INTEGER | 版本 |
| `content` | TEXT | 收藏 XML |
| `source_id` | TEXT | 来源 ID |
| `sync_status` | INTEGER | 同步状态 |
| `upload_status` | INTEGER | 上传状态 |
| `fromusr` | TEXT | 来源用户 |
| `fromusr_id` | INTEGER | 来源用户内部 ID |
| `realchatname` | TEXT | 来源聊天名 |
| `realchatname_id` | INTEGER | 来源聊天内部 ID |
| `ext_buf` | TEXT | 扩展内容 |
| `upload_error_code` | INTEGER | 上传错误码 |
| `trans_res_status` | INTEGER | 转码状态 |
| `trans_res_error_code` | INTEGER | 转码错误码 |
| `star` | INTEGER | 星标 |
| `star_type` | INTEGER | 星标类型 |
| `star_biz_key` | TEXT | 星标业务键 |

### 3.3 收藏类型 type

实际样本出现：

| type | 含义 |
|---:|---|
| 1 | 文本 |
| 2 | 图片 |
| 3 | 语音 |
| 4 | 视频 |
| 5 | 文章 |
| 6 | 位置 |
| 8 | 文件 |
| 14 | 聊天记录 |
| 16 | 笔记 |
| 17 | 小程序 |
| 19 | 名片/卡片 |
| 20 | 视频号/finderFeed |

`type=20` 实际内容：

```xml
<favitem type="20">
  <source ...>
    ...
  </source>
  <finderFeed>
    ...
  </finderFeed>
</favitem>
```

它不是普通卡片，应该展示为视频号。

### 3.4 时间

实际 `update_time` 为 10 位 Unix 秒。代码中可兼容毫秒：

```python
if ts > 9_999_999_999:
    ts = ts // 1000
```

## 4. content XML

收藏的 `content` 是 XML，不是普通文本。

常见结构：

```xml
<favitem type="1">
  <desc>...</desc>
  <source ...>...</source>
</favitem>
```

```xml
<favitem type="2">
  <img .../>
  <source ...>...</source>
</favitem>
```

```xml
<favitem type="14">
  <title>群聊的聊天记录</title>
  <desc>...</desc>
  <recorditem>...</recorditem>
</favitem>
```

```xml
<favitem type="20">
  <finderFeed>...</finderFeed>
</favitem>
```

### 4.1 为什么不能直接 print

`content` 可能包含：

- CDATA；
- `&lt;` 转义嵌套 XML；
- 图片/媒体 URL；
- 聊天记录嵌套 XML。

建议用 XML 解析器或至少 `html.unescape`。

## 5. Python：列出收藏

```python
import sqlite3
from datetime import datetime
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\favorite\favorite.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

sql = """
SELECT local_id, type, update_time, fromusr, realchatname, content
FROM fav_db_item
ORDER BY update_time DESC
LIMIT 20
"""

for local_id, type_code, ts, from_user, chat, content in conn.execute(sql):
    if ts and ts > 9_999_999_999:
        ts //= 1000
    time_text = datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M") if ts else ""
    print(f"{time_text} type={type_code} from={from_user} chat={chat} id={local_id}")
    print(content[:200])

conn.close()
```

## 6. Python：按类型筛选

```python
def favorite_type_name(code: int) -> str:
    return {
        1: "文本", 2: "图片", 3: "语音", 4: "视频",
        5: "文章", 6: "位置", 8: "文件", 14: "聊天记录",
        16: "笔记", 17: "小程序", 19: "名片", 20: "视频号",
    }.get(code, f"其他({code})")

for row in conn.execute("SELECT local_id, type, content FROM fav_db_item WHERE type = ?", (20,)):
    print(row[0], favorite_type_name(row[1]), row[2][:300])
```

## 7. 收藏标签

### 7.1 fav_tag_db_item

```sql
CREATE TABLE fav_tag_db_item(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id INTEGER,
    name TEXT,
    seq INTEGER
);
```

### 7.2 fav_bind_tag_db_item

```sql
CREATE TABLE fav_bind_tag_db_item(
    tag_local_id INTEGER,
    tag_server_id INTEGER,
    fav_local_id INTEGER,
    fav_server_id INTEGER,
    op_code INTEGER,
    CONSTRAINT _TAGLOCALID_FAVLOCALID UNIQUE(tag_local_id, fav_local_id)
);
```

关系：

```text
fav_tag_db_item.local_id
  = fav_bind_tag_db_item.tag_local_id
  = 收藏标签

fav_db_item.local_id
  = fav_bind_tag_db_item.fav_local_id
  = 收藏条目
```

### 7.3 Python：收藏 + 标签

```python
sql = """
SELECT f.local_id, f.type, t.name
FROM fav_db_item f
LEFT JOIN fav_bind_tag_db_item b ON b.fav_local_id = f.local_id
LEFT JOIN fav_tag_db_item t ON t.local_id = b.tag_local_id
ORDER BY f.update_time DESC
LIMIT 20
"""
for row in conn.execute(sql):
    print(row)
```

## 8. Name2Id 和其他表

### 8.1 Name2Id

```sql
CREATE TABLE Name2Id(
    user_name TEXT PRIMARY KEY
);
```

`fromusr_id`、`realchatname_id` 可能引用本表或内部 ID；实际使用时建议先按字符串 `fromusr`/`realchatname` 展示，再用 ID 做辅助。

### 8.2 config / buff

```sql
CREATE TABLE config(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);
```

`config` 和 `buff` 都是通用键值表，值四选一：

```text
ValueInt64
ValueDouble
ValueStdStr
ValueBlob
```

### 8.3 resync_task

```sql
CREATE TABLE resync_task(
    task_id INTEGER PRIMARY KEY,
    resync_version INTEGER,
    key_buf TEXT,
    selector INTEGER,
    need_resync INTEGER
);
```

同步任务状态。

### 8.4 sync_client_storage

```sql
CREATE TABLE sync_client_storage(
    key INTEGER PRIMARY KEY,
    value TEXT,
    value_version INTEGER
);
```

同步客户端状态。

## 9. 媒体文件

收藏的图片/语音/视频/文件通常有：

- `cdn_url`；
- `thumb_url`；
- `aes_key`；
- 文件大小；
- 文件 MD5。

实际媒体文件可能不在 `favorite.db` 内，需要：

- 从 URL 下载；
- 从本地缓存查找；
- 用 `hardlink.db` / 文件系统匹配。

## 10. 常见问题

### 10.1 收藏时间显示 1970

`update_time` 可能为 0 或毫秒。先判断：

```python
if ts > 9_999_999_999:
    ts //= 1000
```

### 10.2 内容全是 XML

正常。收藏 `content` 就是 XML，需要解析。

### 10.3 语音/视频打不开

收藏表通常只存 URL/MD5，不存二进制。需要下载或从本地缓存找。

### 10.4 type=20 显示成“其他”

type=20 是视频号/finderFeed，需要单独映射。

### 10.5 收藏列表为空

- 打开错库；
- 当前账号没有收藏；
- 表名不是 `fav_db_item`（版本变体）。

## 11. 速查

```text
favorite/favorite.db
├── fav_db_item            收藏主表
├── fav_tag_db_item        标签
├── fav_bind_tag_db_item   收藏-标签关系
├── Name2Id                内部 ID
├── resync_task            同步任务
├── sync_client_storage    同步状态
├── config / buff          键值配置
└── sqlite_sequence
```

关键点：

1. `content` 是 XML；
2. type=20 是视频号；
3. 时间可能是秒或毫秒；
4. 媒体通常只有 URL/MD5，不是 BLOB；
5. `fromusr`/`realchatname` 是最直观的来源信息。
