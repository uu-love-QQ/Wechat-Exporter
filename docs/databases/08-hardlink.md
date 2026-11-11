# hardlink/hardlink.db：媒体文件路径索引

## 1. 这个库是做什么的

`hardlink/hardlink.db` 把消息里的 **MD5** 映射到本地真实文件：

```text
消息 XML 里的 MD5
   -> hardlink 表
   -> file_name + dir1 + dir2
   -> 本地图片/视频/文件
```

它主要解决：

- 消息里只有 MD5，没有完整路径；
- 文件被移动到其他月份目录；
- 图片/视频/文件使用不同的索引表；
- 需要从 `msg/attach`、`msg/video`、`FileStorage` 找文件。

## 2. 实际表清单

本次样本 8 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `dir2id` | 280 | 目录/身份索引 |
| `image_hardlink_info_v4` | 30662 | 图片索引 |
| `video_hardlink_info_v4` | 166 | 视频索引 |
| `file_hardlink_info_v4` | 661 | 文件索引 |
| `file_checkpoint_v4` | 0 | 文件检查点 |
| `video_checkpoint_v4` | 0 | 视频检查点 |
| `talker_checkpoint_v4` | 0 | 会话检查点 |
| `db_info` | 4 | 数据库信息 |

## 3. 实际 DDL

### 3.1 dir2id

```sql
CREATE TABLE dir2id(
    username TEXT PRIMARY KEY
);
```

虽然字段叫 `username`，实际是混合内容：

- 月份：`2024-12`、`2025-01`；
- 会话/目录哈希：`04ddd9ee...`。

`dir2id.rowid` 是数字索引，被其他表的 `dir1`/`dir2` 引用。

实际样本：

```text
rowid=1  -> 2024-12
rowid=2  -> 2025-01
...
rowid=24 -> 00f877f7e7c1dc308e4acf1debe47014
rowid=25 -> 0233de66c93eafc915b64e0fd19d77e1
```

### 3.2 文件索引表

```sql
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
```

字段：

| 字段 | 作用 |
|---|---|
| `md5_hash` | 32 位哈希，用于快速查询 |
| `md5` | 完整 32 位 Hex MD5 |
| `type` | 1 文件、2 图片、3 视频 |
| `file_name` | 本地文件名 |
| `file_size` | 文件大小 |
| `modify_time` | 修改时间，Unix 秒 |
| `dir1` | `dir2id.rowid`，通常是会话/目录哈希 |
| `dir2` | `dir2id.rowid`，通常是月份 |
| `_rowid_` | 表内主键 |
| `extra_buffer` | 额外二进制 |

### 3.3 checkpoint 表

```sql
CREATE TABLE file_checkpoint_v4(month_id INTEGER PRIMARY KEY ASC);
CREATE TABLE video_checkpoint_v4(month_id INTEGER PRIMARY KEY ASC);

CREATE TABLE talker_checkpoint_v4(
    talker_id INTEGER PRIMARY KEY ASC,
    month_id INTEGER
);
```

用于增量扫描/索引检查点。

### 3.4 db_info

```sql
CREATE TABLE db_info(
    Key TEXT PRIMARY KEY,
    ValueInt64 INTEGER,
    ValueDouble REAL,
    ValueStdStr TEXT,
    ValueBlob BLOB
);
```

通用键值信息。

## 4. 路径怎么还原

实际路径通常类似：

```text
<账号目录>/msg/attach/<dir1>/<dir2>/Img/<file_name>
<账号目录>/msg/video/<dir1>/<dir2>/<file_name>
<账号目录>/FileStorage/File/<dir1>/<dir2>/<file_name>
```

但不同版本/文件类型路径不完全一样。

最重要的步骤：

1. 用消息里的 MD5 查对应表；
2. 得到 `dir1`、`dir2`；
3. 用 `dir2id` 把数字换成字符串；
4. 拼 `file_name`；
5. 在真实文件系统中验证文件存在。

### 4.1 Python：解析 MD5

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\hardlink\hardlink.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

def resolve(md5: str, table: str):
    row = conn.execute(
        f'SELECT file_name, dir1, dir2, type, file_size FROM "{table}" WHERE md5 = ?1 LIMIT 1',
        (md5,),
    ).fetchone()
    if not row:
        return None

    file_name, dir1, dir2, type_code, size = row

    def dir_name(index: int) -> str:
        found = conn.execute(
            "SELECT username FROM dir2id WHERE rowid = ?1", (index,)
        ).fetchone()
        return found[0] if found else str(index)

    return {
        "file_name": file_name,
        "dir1": dir_name(dir1),
        "dir2": dir_name(dir2),
        "type": type_code,
        "size": size,
    }

print(resolve("67ed34697e96f6a6c87b752777a293fd", "image_hardlink_info_v4"))
```

### 4.2 Python：在账号目录中找文件

```python
from pathlib import Path

account = Path(r"C:\Users\你\Downloads\wechat_account")
info = resolve("67ed34697e96f6a6c87b752777a293fd", "image_hardlink_info_v4")

if info:
    candidates = [
        account / "msg" / "attach" / info["dir1"] / info["dir2"] / "Img" / info["file_name"],
        account / "msg" / "attach" / info["dir1"] / info["dir2"] / info["file_name"],
        account / "msg" / "video" / info["dir1"] / info["dir2"] / info["file_name"],
        account / "FileStorage" / "File" / info["dir1"] / info["dir2"] / info["file_name"],
    ]
    for path in candidates:
        if path.exists():
            print("找到：", path)
            break
```

## 5. 按类型选表

| 消息内容 | 表 |
|---|---|
| 图片 | `image_hardlink_info_v4` |
| 视频 | `video_hardlink_info_v4` |
| 文件/附件 | `file_hardlink_info_v4` |

实际 `type`：

```text
1 文件
2 图片
3 视频
```

不要只用 `type` 判断，表名更可靠。

## 6. 实际样本

```text
image:
md5=67ed34697e96f6a6c87b752777a293fd
file_name=0767676014169d7df0282c7c73277d83_t_W.dat
dir1=26
dir2=15

video:
md5=a66ad958e34c467a8631cc1ca0ada5e6
file_name=4f2e82aed875e67adeced9523d97f12e.mp4
dir1=13
dir2=0

file:
md5=2778c88ada1481027ea648f0a7175032
file_name=2023级物理学一班.xlsx
```

`dir1=26` 对应 dir2id 的 MD5，`dir2=15` 对应月份。

## 7. 常见问题

### 7.1 表里有 MD5，但文件不存在

- 文件可能被清理；
- 可能在别的账号目录；
- 文件名可能被重命名；
- 需要检查不同路径模板。

### 7.2 MD5 有多个记录

同一内容可能被多处引用。优先选择：

```text
modify_time 最新
file_size 合理
file_name 与消息扩展名一致
```

### 7.3 dir1/dir2 无法解析

说明 `dir2id` 缺少对应 rowid，或版本不同。直接打印数字，继续尝试文件系统。

### 7.4 视频文件名不是 mp4

`file_name` 可能没有扩展名或带 `.dat`；实际文件类型需要看文件头。

### 7.5 图片是缩略图

消息 XML 和 hardlink 可能同时有原图/缩略图。根据文件大小、文件名后缀（`_t`、`_h`）和实际尺寸判断。

## 8. 速查

```text
hardlink/hardlink.db
├── dir2id                    数字索引 -> 月份/会话哈希
├── image_hardlink_info_v4    图片
├── video_hardlink_info_v4    视频
├── file_hardlink_info_v4     文件
├── *checkpoint_v4            扫描检查点
└── db_info                   键值
```

关键点：

1. `md5` 是关联键；
2. `dir1/dir2` 指向 `dir2id.rowid`；
3. `dir2id` 混合月份和哈希；
4. 路径要按账号目录 + 文件类型尝试；
5. 数据库只给文件名/索引，文件可能已被清理。
