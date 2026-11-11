# FTS 数据库：contact_fts / favorite_fts / message_fts

## 1. FTS 是什么

FTS = Full Text Search，全文搜索索引。

微信的：

```text
contact/contact_fts.db
favorite/favorite_fts.db
message/message_fts.db
```

不是普通数据表，而是 SQLite FTS5 虚表 + 影子表 + 微信自定义 tokenizer。

## 2. 为什么普通 Python 查询会报错

```python
import sqlite3
conn = sqlite3.connect("file:message/message_fts.db?mode=ro", uri=True)
conn.execute("SELECT * FROM message_fts_v4_0").fetchall()
```

可能报：

```text
sqlite3.OperationalError: no such tokenizer: MMFtsTokenizer
```

原因：

- 建表语句是：

```sql
CREATE VIRTUAL TABLE message_fts_v4_0
USING fts5(
    tokenize = 'MMFtsTokenizer disable_pinyin',
    ...
);
```

- `MMFtsTokenizer` 是微信自己实现的 tokenizer；
- 普通 SQLite/Python 没有这个 tokenizer；
- 没有 tokenizer 就不能打开/查询虚表。

## 3. 实际 FTS 结构

### 3.1 contact_fts.db

实际 37 张表，包括：

```text
chatroom_member_fts_v3
chatroom_member_fts_v3_aux
chatroom_member_fts_v3_config
chatroom_member_fts_v3_content
chatroom_member_fts_v3_data
chatroom_member_fts_v3_docsize
chatroom_member_fts_v3_idx

contact_fts_v5
contact_fts_v5_config
contact_fts_v5_content
contact_fts_v5_data
contact_fts_v5_docsize
contact_fts_v5_idx

contact_fts_pinyin_v5
contact_fts_pinyin_v5_config
contact_fts_pinyin_v5_data
contact_fts_pinyin_v5_docsize
contact_fts_pinyin_v5_idx

search_dict_fts_v1
search_dict_fts_v1_config
search_dict_fts_v1_data
search_dict_fts_v1_docsize
search_dict_fts_v1_idx
search_dict_v1

wa_contact_fts_v1
wa_contact_fts_v1_config
wa_contact_fts_v1_content
wa_contact_fts_v1_data
wa_contact_fts_v1_docsize
wa_contact_fts_v1_idx
wa_contact_fts_pinyin_v1
wa_contact_fts_pinyin_v1_config
wa_contact_fts_pinyin_v1_data
wa_contact_fts_pinyin_v1_docsize
wa_contact_fts_pinyin_v1_idx

db_info
name2id
```

### 3.2 favorite_fts.db

```text
fav_fts_v1
fav_fts_v1_config
fav_fts_v1_content
fav_fts_v1_data
fav_fts_v1_docsize
fav_fts_v1_idx
table_info
```

建表：

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

### 3.3 message_fts.db

实际 60 张表，包括：

```text
message_fts_v4_0
message_fts_v4_0_config
message_fts_v4_0_content
message_fts_v4_0_data
message_fts_v4_0_docsize
message_fts_v4_0_idx

message_fts_v4_1
...
message_fts_v4_aux_0
message_fts_v4_range
message_fts_v4_session_delete_info

ImgFts0V0 ... ImgFts3V0
ImgFtsAux0V0 ... ImgFtsAux3V0
ImgRangeV0
deleteImgFtsV0
ImgTableInfo
name2id
table_info
```

消息 FTS 建表：

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

## 4. 虚表和影子表的关系

FTS5 通常生成：

```text
<fts>             虚表
<fts>_config      配置
<fts>_content     内容
<fts>_data        倒排数据块
<fts>_docsize     文档大小
<fts>_idx         倒排索引
```

例如：

```text
message_fts_v4_0
message_fts_v4_0_config
message_fts_v4_0_content
message_fts_v4_0_data
message_fts_v4_0_docsize
message_fts_v4_0_idx
```

不能只复制虚表而丢掉影子表；否则索引损坏。

## 5. 能做什么、不能做什么

### 5.1 不能直接做

```python
# 普通 SQLite 通常会失败
conn.execute("SELECT * FROM message_fts_v4_0 WHERE message_fts_v4_0 MATCH 'hello'")
```

### 5.2 可以做的替代方案

1. 用 `wechat-exporter search` 搜索；
2. 直接对消息表做 `LIKE`；
3. 读取 FTS 的影子表（只能拿原始数据，不能正常分词）；
4. 如果要自己搜索，重新建立普通 FTS 索引。

### 5.3 直接 LIKE 搜索消息

```python
import sqlite3
from pathlib import Path

root = Path(r"C:\Users\你\Downloads\db_decrypted")
keyword = "hello"

for db_path in (root / "message").glob("message_*.db"):
    if db_path.name.endswith("_fts.db") or db_path.name == "message_resource.db":
        continue
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    tables = [
        name for (name,) in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'Msg_%'"
        )
    ]
    for table in tables:
        try:
            rows = conn.execute(
                f'SELECT local_id, message_content FROM "{table}" WHERE message_content LIKE ? LIMIT 3',
                (f"%{keyword}%",),
            ).fetchall()
            for row in rows:
                print(db_path.name, table, row[0])
        except sqlite3.Error:
            pass
    conn.close()
```

注意：

- `message_content` 可能是 zstd/BLOB，LIKE 只能查到未压缩文本；
- 更可靠的是先解码再搜索；
- `wechat-exporter search` 已经处理了压缩和分片。

## 6. Python：读取影子表

虚表打不开，但影子表是普通表，可以读：

```python
import sqlite3
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\message\message_fts.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

# 注意：不同版本表名可能不同
rows = conn.execute("""
    SELECT id, c0, c1, c2, c3, c4, c5, c6
    FROM message_fts_v4_0_content
    LIMIT 10
""").fetchall()

for row in rows:
    print(row)
```

`c0` 是 `acontent`，`c1` 是 `message_local_id`，后续字段按建表顺序。

这不是“全文搜索”，只是读取索引内容。

## 7. 备份/恢复注意

- 备份必须包含虚表 + 所有影子表；
- 只恢复消息表，不恢复 FTS，会导致搜索缺失；
- 单独恢复 FTS 而不恢复消息表，会索引到不存在的消息；
- 最安全的方法是整库 `decrypt` 备份，不要手工复制单表。

## 8. 为什么微信要用自定义 tokenizer

中文搜索需要：

- 分词；
- 拼音；
- 特殊字符；
- 数字 token；
- 多音字/简繁等。

`MMFtsTokenizer` 参数：

```text
disable_pinyin
disable_origin
enable_special_char
enable_num_token
```

普通 SQLite 的 `unicode61` 不能完全复现这些规则。

## 9. 常见问题

### 9.1 FTS 文件很大

正常，索引通常比消息正文还大。

### 9.2 删除 FTS 后搜索还能用吗

微信可能重建；但不要手工删，除非你明确知道后果。

### 9.3 `no such tokenizer` 能绕过吗

不能直接绕过。可以：
- 使用微信环境/工具；
- 或自己导出消息后重建普通 FTS。

### 9.4 影子表内容看不懂

正常，它是倒排索引数据，不是给人类直接读的。

## 10. 速查

```text
*_fts.db
├── <fts>            FTS5 虚表（需要微信 tokenizer）
├── <fts>_config     配置
├── <fts>_content    内容
├── <fts>_data       倒排块
├── <fts>_docsize    文档大小
└── <fts>_idx        倒排索引
```

关键点：

1. 普通 SQLite 不能直接查虚表；
2. 影子表必须一起备份；
3. 恢复消息后 FTS 不会自动更新；
4. 搜索优先用 `wechat-exporter search`；
5. 不要按普通表修改 FTS。
