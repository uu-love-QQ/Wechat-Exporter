# head_image/head_image.db：头像缓存

## 1. 这个库是做什么的

`head_image/head_image.db` 保存联系人/群头像的本地缓存。

本次样本只有 1 张表，1369 行。

## 2. 实际 DDL

```sql
CREATE TABLE head_image(
    username TEXT PRIMARY KEY,
    md5 TEXT,
    image_buffer BLOB,
    update_time INTEGER
);
```

## 3. 字段解释

| 字段 | 类型 | 含义 |
|---|---|---|
| `username` | TEXT | 联系人/群 ID |
| `md5` | TEXT | 头像内容 MD5 |
| `image_buffer` | BLOB | 头像二进制 |
| `update_time` | INTEGER | 更新时间，Unix 秒 |

实际 `image_buffer` 开头：

```text
ff d8 ff e0 00 10 4a 46 ...
```

这是 JPEG 文件。不要按文本解码。

## 4. Python：导出所有头像

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\head_image\head_image.db")
out = Path("avatars")
out.mkdir(exist_ok=True)

conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

for username, md5, image, update_time in conn.execute(
    "SELECT username, md5, image_buffer, update_time FROM head_image"
):
    if not image:
        continue
    safe_name = username.replace("/", "_").replace("\\", "_")
    path = out / f"{safe_name}.jpg"
    path.write_bytes(image)
    print(username, len(image), "->", path)
```

## 5. Python：查某个人的头像

```python
username = "wxid_xxx"
row = conn.execute(
    "SELECT md5, image_buffer, update_time FROM head_image WHERE username = ?1",
    (username,),
).fetchone()

if row:
    md5, image, update_time = row
    Path("avatar.jpg").write_bytes(image)
    print("MD5", md5, "update", update_time)
else:
    print("没有缓存")
```

## 6. 头像从哪里来

`contact` 表里也有：

```text
big_head_url
small_head_url
head_img_md5
```

三种来源：

1. 网络 URL（可能失效）；
2. `head_image.db` 本地 BLOB；
3. 微信资源/缓存目录。

本地 `head_image.db` 最稳定，优先使用。

## 7. 实际样本

```text
username=wxid_xxx
md5=eaea73c8f12df98a68a7ac92eebf1cc7
image_buffer=5193 bytes
开头=ffd8ffe0...（JPEG）
```

群头像也存在，`username` 是 `xxx@chatroom`。

## 8. 常见问题

### 8.1 头像是损坏的

- 检查开头是否为 `ff d8 ff`（JPEG）；
- 可能是 WebP/PNG，检查文件头；
- 不要按 UTF-8 解码。

### 8.2 找不到某联系人头像

- 可能未缓存；
- 可能在 `contact.big_head_url`；
- 可能已过期/被清理；
- 可以尝试微信资源目录。

### 8.3 头像很多，怎么批量导出

使用前面的循环，并跳过空 BLOB：

```python
if not image:
    continue
```

`username` 可能包含 `/`、`\`、`@`，文件名要替换非法字符。

## 9. 速查

```text
head_image/head_image.db
└── head_image
    ├── username      wxid / @chatroom
    ├── md5           头像 MD5
    ├── image_buffer  JPEG BLOB
    └── update_time   Unix 秒
```

关键点：

1. 头像是 BLOB，不是文本；
2. 大多数是 JPEG；
3. `username` 可能包含特殊字符；
4. 本地缓存比网络 URL 更稳定；
5. 空 BLOB 要跳过。
