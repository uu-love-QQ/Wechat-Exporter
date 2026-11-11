# emoticon/emoticon.db：表情包

## 1. 这个库是做什么的

`emoticon/emoticon.db` 保存微信表情包：

- 收藏的表情；
- 最近使用的表情；
- 表情商店的表情包；
- 表情文件在包内的偏移；
- 表情 CDN/加密下载地址；
- 表情名称和作者。

它不保存聊天消息。聊天里收到表情时，消息表通常只有 MD5/URL，需要来这里或本地文件查找。

## 2. 实际表清单

本次样本 7 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `kCustomEmoticonOrderTable` | 0 | 自定义表情排序 |
| `kExpressRecentUseEemoticonTable` | 1 | 最近使用 |
| `kFavEmoticonOrderTable` | 333 | 收藏表情排序 |
| `kNonStoreEmoticonTable` | 333 | 非商店/收藏表情 |
| `kStoreEmoticonCaptionsTable` | 692 | 商店表情名称 |
| `kStoreEmoticonFilesTable` | 32 | 商店表情文件 |
| `kStoreEmoticonPackageTable` | 2 | 商店表情包 |

## 3. 完整 DDL

### 3.1 kCustomEmoticonOrderTable

```sql
CREATE TABLE kCustomEmoticonOrderTable(
    md5 TEXT
);
```

自定义表情排序，按行顺序。

### 3.2 kExpressRecentUseEemoticonTable

```sql
CREATE TABLE kExpressRecentUseEemoticonTable(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);
```

最近使用表情的键值表。

### 3.3 kFavEmoticonOrderTable

```sql
CREATE TABLE kFavEmoticonOrderTable(
    md5 TEXT
);
```

收藏表情排序。

### 3.4 kNonStoreEmoticonTable

```sql
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
```

字段：

| 字段 | 作用 |
|---|---|
| `type` | 表情类型 |
| `md5` | 表情内容 MD5 |
| `caption` | 名称/说明 |
| `product_id` | 产品 ID |
| `aes_key` | 加密资源密钥 |
| `thumb_url` | 缩略图 URL |
| `tp_url` | 缩略图/预览 URL |
| `auth_key` | 鉴权 key |
| `cdn_url` | CDN 下载地址 |
| `extern_url` | 外部下载地址 |
| `extern_md5` | 外部文件 MD5 |
| `encrypt_url` | 加密下载地址 |
| `designer_id` | 设计师 ID |
| `activity_id` | 活动 ID |

### 3.5 kStoreEmoticonCaptionsTable

```sql
CREATE TABLE kStoreEmoticonCaptionsTable(
    package_id_ TEXT,
    md5_ TEXT,
    language_ TEXT,
    caption_ TEXT
);
```

商店表情的本地化名称：

```text
package_id_ + md5_ + language_  -> caption_
```

### 3.6 kStoreEmoticonFilesTable

```sql
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
```

字段：

| 字段 | 作用 |
|---|---|
| `package_id_` | 表情包 ID |
| `md5_` | 表情 MD5 |
| `type_` | 文件类型 |
| `sort_order_` | 排序 |
| `emoticon_size_` | 表情数据大小 |
| `emoticon_offset_` | 表情数据在包内偏移 |
| `thumb_size_` | 缩略图大小 |
| `thumb_offset_` | 缩略图偏移 |

表情包文件通常是打包文件，需要按 `offset/size` 切出来。

### 3.7 kStoreEmoticonPackageTable

```sql
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

| 字段 | 作用 |
|---|---|
| `package_id_` | 表情包 ID |
| `package_name_` | 名称 |
| `payment_status_` | 付费状态 |
| `download_status_` | 下载状态 |
| `install_time_` | 安装时间 |
| `remove_time_` | 移除时间 |
| `sort_order_` | 排序 |
| `introduction_` | 简介 |
| `full_description_` | 完整描述 |
| `copyright_` | 版权 |
| `author_` | 作者 |
| `store_icon_url_` | 商店图标 |
| `panel_url_` | 面板 URL |

## 4. Python：列出收藏表情

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\emoticon\emoticon.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

sql = """
SELECT type, md5, caption, cdn_url, encrypt_url, extern_url
FROM kNonStoreEmoticonTable
LIMIT 20
"""

for row in conn.execute(sql):
    print(row)
```

## 5. Python：列出商店表情包

```python
sql = """
SELECT package_id_, package_name_, author_, store_icon_url_
FROM kStoreEmoticonPackageTable
ORDER BY sort_order_
"""

for package_id, name, author, icon in conn.execute(sql):
    print(package_id, name, author, icon)
```

## 6. Python：查某个表情的文件信息

```python
md5 = "你的表情MD5"

# 非商店/收藏表情
for row in conn.execute(
    """
    SELECT type, md5, caption, cdn_url, encrypt_url, extern_url
    FROM kNonStoreEmoticonTable
    WHERE md5 = ?1
    """,
    (md5,),
):
    print("non-store:", row)

# 商店表情
for row in conn.execute(
    """
    SELECT package_id_, md5_, emoticon_size_, emoticon_offset_,
           thumb_size_, thumb_offset_
    FROM kStoreEmoticonFilesTable
    WHERE md5_ = ?1
    """,
    (md5,),
):
    print("store:", row)
```

## 7. 表情文件和聊天消息如何关联

聊天消息中的表情通常是：

```text
local_type = 47
message_content = [动画表情]
```

XML 里可能有：

```xml
<emoji md5="..." cdnurl="..." encrypturl="..." .../>
```

拿到 `md5` 后：

1. 查 `kNonStoreEmoticonTable`；
2. 查 `kStoreEmoticonFilesTable`；
3. 查本地表情包文件；
4. 如果只有 URL，按 `cdn_url`/`encrypt_url` 下载；
5. 如果 `encrypt_url` 有 `aes_key`，需要 AES 解密。

## 8. 实际样本

```text
type=3
md5=ebe91005c3f0bd7afa0ef150df667fc4
caption 为空
cdn_url/extern_url/encrypt_url 长度约 284
```

这说明：

- `caption` 可以为空；
- URL 可能三个都存在；
- 不要假设 `caption` 一定可读；
- `md5` 是最可靠的关联键。

## 9. 常见问题

### 9.1 表情显示成 [动画表情]

说明只拿到了占位文本，没有找到文件或 URL。需要查：
- 消息 XML 里的 `<emoji>`；
- `kNonStoreEmoticonTable`；
- 本地表情文件。

### 9.2 表情文件偏移怎么用

```python
with open(package_file, "rb") as f:
    f.seek(emoticon_offset_)
    data = f.read(emoticon_size_)
```

缩略图同理：

```python
f.seek(thumb_offset_)
thumb = f.read(thumb_size_)
```

### 9.3 cdn_url / encrypt_url / extern_url 区别

- `cdn_url`：普通 CDN；
- `encrypt_url`：加密下载；
- `extern_url`：外部链接；
- 需要结合 `aes_key`/`auth_key` 和实际 HTTP 返回判断。

### 9.4 表情包文件找不到

可能被清理、未下载、或在其他缓存目录。数据库只保存元数据。

## 10. 速查

```text
emoticon/emoticon.db
├── kNonStoreEmoticonTable      收藏/非商店表情
├── kFavEmoticonOrderTable      收藏排序
├── kCustomEmoticonOrderTable   自定义排序
├── kExpressRecentUse...        最近使用
├── kStoreEmoticonPackageTable  商店表情包
├── kStoreEmoticonFilesTable    包内文件偏移
└── kStoreEmoticonCaptionsTable 名称/语言
```

关键点：

1. `md5` 是关联键；
2. `type` 是表情类型；
3. 商店表情按 offset/size 从包文件切；
4. 收藏表情可能是 URL，不一定有本地文件；
5. `caption` 可能为空。
