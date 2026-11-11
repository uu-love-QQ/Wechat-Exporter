# solitaire / third_app_icon：群接龙与第三方图标

## 1. 两个库分别做什么

### solitaire/solitaire.db

群接龙数据：

- 接龙主内容；
- 有效条目；
- 折叠内容；
- 按群/会话分表。

### third_app_icon/third_app_icon.db

第三方应用/小程序的图标缓存。

## 2. solitaire/solitaire.db

### 2.1 表名规则

每个群/会话一组表：

```text
Solitaire_<hash>
SolitaireValid_<hash>
SolitaireFold_<hash>
```

`<hash>` 是会话/群相关哈希，具体算法随版本可能不同。实际样本有两组：

```text
026ab6ae7b75dc93e03aba06d9e79de1
b69963276a6d1b8ce6dfada7a0517a65
```

### 2.2 Solitaire_<hash>

```sql
CREATE TABLE Solitaire_<hash>(
    header TEXT,
    footer TEXT,
    content TEXT,
    solitaire_string TEXT,
    active_time INTEGER,
    CONSTRAINT SOLITAIRE_UNIQUE_INDEX UNIQUE(header, footer)
);
```

| 字段 | 作用 |
|---|---|
| `header` | 接龙头部 |
| `footer` | 接龙尾部 |
| `content` | 接龙正文 |
| `solitaire_string` | 原始接龙字符串 |
| `active_time` | 时间，Unix 秒 |

### 2.3 SolitaireValid_<hash>

```sql
CREATE TABLE SolitaireValid_<hash>(
    title TEXT,
    valid INTEGER,
    local_id INTEGER,
    sort_seq INTEGER,
    create_time INTEGER,
    CONSTRAINT SOLITAIRE_VALID_UNIQUE_INDEX UNIQUE(local_id, sort_seq)
);
```

| 字段 | 作用 |
|---|---|
| `title` | 条目 |
| `valid` | 是否有效 |
| `local_id` | 消息 local_id |
| `sort_seq` | 排序 |
| `create_time` | 时间 |

### 2.4 SolitaireFold_<hash>

```sql
CREATE TABLE SolitaireFold_<hash>(
    front_content TEXT,
    behind_content TEXT,
    local_id INTEGER,
    sort_seq INTEGER,
    create_time INTEGER,
    CONSTRAINT SOLITAIRE_FOLD_UNIQUE_INDEX UNIQUE(local_id, sort_seq)
);
```

| 字段 | 作用 |
|---|---|
| `front_content` | 折叠前内容 |
| `behind_content` | 折叠后内容 |
| `local_id` | 消息 local_id |
| `sort_seq` | 排序 |
| `create_time` | 时间 |

### 2.5 Python：列出接龙表并读取

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\solitaire\solitaire.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

tables = [
    name
    for (name,) in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'Solitaire_%'"
    )
]
print(tables)

for table in tables:
    print("=== ", table)
    for row in conn.execute(f'SELECT * FROM "{table}" LIMIT 5'):
        print(row)
```

### 2.6 Python：读取某个群接龙

```python
def read_solitaire(conn, hash_value: str):
    for prefix in ("Solitaire", "SolitaireValid", "SolitaireFold"):
        table = f"{prefix}_{hash_value}"
        try:
            rows = conn.execute(f'SELECT * FROM "{table}"').fetchall()
            print(prefix, len(rows))
            for row in rows[:5]:
                print("  ", row)
        except sqlite3.Error as error:
            print(prefix, "不存在/为空：", error)
```

### 2.7 实际样本

```text
Solitaire_026...: 1
SolitaireValid_026...: 9
SolitaireFold_026...: 7
Solitaire_b699...: 1
SolitaireValid_b699...: 16
SolitaireFold_b699...: 14
```

### 2.8 常见问题

- hash 无法直接反推 wxid，需要结合会话/上下文；
- `content` 可能是 XML/文本；
- `valid=0` 的条目不一定是错误，可能是历史版本；
- `sort_seq` 可能不是小整数，按排序键处理。

## 3. third_app_icon/third_app_icon.db

### 3.1 实际 DDL

```sql
CREATE TABLE third_app_icon(
    app_key TEXT PRIMARY KEY,
    icon_url_md5 TEXT,
    image_buffer BLOB,
    update_time INTEGER
);
```

字段：

| 字段 | 作用 |
|---|---|
| `app_key` | 第三方应用/小程序键 |
| `icon_url_md5` | 图标 URL/内容 MD5 |
| `image_buffer` | 图标二进制 |
| `update_time` | 更新时间 |

本次样本表存在但 0 行。

### 3.2 Python：导出图标

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\third_app_icon\third_app_icon.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

for app_key, md5, image, update_time in conn.execute(
    "SELECT app_key, icon_url_md5, image_buffer, update_time FROM third_app_icon"
):
    if not image:
        continue
    path = Path("icons") / f"{app_key}.png"
    path.parent.mkdir(exist_ok=True)
    path.write_bytes(image)
    print(app_key, md5, len(image), update_time)
```

图片格式可能是 PNG/JPEG，先看文件头，不要强行用 `.png`。

### 3.3 常见问题

- 表为空是正常；
- `app_key` 可能是包名/小程序 ID；
- `image_buffer` 是 BLOB；
- 与聊天消息中的小程序消息不是同一张表。

## 4. 速查

```text
solitaire/solitaire.db
├── Solitaire_<hash>       接龙正文
├── SolitaireValid_<hash>  有效条目
└── SolitaireFold_<hash>   折叠内容

third_app_icon/third_app_icon.db
└── third_app_icon
    ├── app_key
    ├── icon_url_md5
    ├── image_buffer
    └── update_time
```

关键点：

1. solitaire 按群/会话分表；
2. `hash` 需结合上下文；
3. `active_time`/`create_time` 通常是秒；
4. 第三方图标是 BLOB；
5. 空表不代表数据库损坏。
