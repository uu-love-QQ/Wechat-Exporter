# sns/sns.db：朋友圈、点赞、评论

## 1. 这个库是做什么的

`sns/sns.db` 保存微信朋友圈：

- 朋友圈正文；
- 图片/视频/链接；
- 点赞；
- 评论；
- 置顶；
- 草稿；
- 发布任务；
- 广告；
- 朋友圈断点/同步状态。

聊天记录不在这个库。

## 2. 实际表清单

本次样本 13 张表：

| 表 | 行数 | 作用 |
|---|---:|---|
| `SnsTimeLine` | 1950 | 朋友圈主时间线 |
| `SnsMessage_tmp3` | 783 | 评论/点赞互动 |
| `SnsTopItem_1` | 20310 | 置顶/顶部条目 |
| `SnsMainTimeLineBreakFlag` | 1439 | 主时间线断点 |
| `SnsUserTimeLineBreakFlagV2` | 390 | 用户时间线断点 |
| `SnsAdTimeLine` | 1 | 广告时间线 |
| `SnsDraft` | 0 | 草稿 |
| `SnsPublishTask` | 0 | 发布任务 |
| `SnsNoteVoice` | 0 | 语音笔记 |
| `SnsErrorMessage` | 0 | 错误记录 |
| `SnsIgnoredDataItem` | 0 | 忽略项 |
| `SnsPendingDraftDeletionTable` | 0 | 待删除草稿 |
| `sqlite_sequence` | 1 | 自增序列 |

## 3. 朋友圈主表 SnsTimeLine

### 3.1 实际 DDL

```sql
CREATE TABLE SnsTimeLine(
    tid INTEGER PRIMARY KEY DESC,
    user_name TEXT,
    content TEXT,
    pack_info_buf TEXT
);
```

### 3.2 字段

| 字段 | 作用 |
|---|---|
| `tid` | 时间线 ID |
| `user_name` | 发布者 wxid |
| `content` | 朋友圈 XML |
| `pack_info_buf` | 二进制/压缩打包信息 |

### 3.3 tid 可能为负数

实际样本：

```text
tid = -3395851328022302004
```

这是 64 位有符号整数，不是错误。Python 的 `int` 可以正常处理；不要转成 unsigned 后排序。

### 3.4 content 的实际结构

```xml
<SnsDataItem>
  <TimelineObject>
    <id>...</id>
    <username>wxid_xxx</username>
    <createTime>1794206232</createTime>
    <contentDesc>正文...</contentDesc>
    <contentDescShowType>0</contentDescShowType>
    <objectDesc>
      <mediaList>...</mediaList>
    </objectDesc>
    <likeUserList>...</likeUserList>
    <commentList>...</commentList>
  </TimelineObject>
</SnsDataItem>
```

实际内容可能包含：

- 图片；
- 视频；
- 链接；
- 位置；
- 转发；
- 广告；
- 点赞/评论列表；
- 转义 XML。

## 4. SnsMessage_tmp3：互动表

### 4.1 实际 DDL

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

### 4.2 字段

| 字段 | 作用 |
|---|---|
| `local_id` | 互动本地 ID |
| `create_time` | 互动时间，Unix 秒 |
| `type` | 互动类型 |
| `feed_id` | 关联 `SnsTimeLine.tid` |
| `is_unread` | 是否未读 |
| `from_username` | 发起人 |
| `from_nickname` | 发起人昵称 |
| `to_username` | 被互动人 |
| `to_nickname` | 被互动人昵称 |
| `content` | 评论内容 |
| `serialized_comment_buf` | 评论序列化 BLOB |
| `serialized_ref_buf` | 引用序列化 BLOB |
| `comment_id` | 评论 ID |
| `client_id` | 客户端 ID |
| `comment64_id` | 64 位评论 ID |
| `comment_flag` | 评论标志 |
| `del_status` | 删除状态 |
| `is_relative_me` | 是否与我相关 |

实际 `type` 样本：

```text
1
2
4
```

不要假设只有“点赞/评论”两种。

### 4.3 feed_id 关联

```sql
SELECT s.tid, s.user_name, m.type, m.from_username, m.content
FROM SnsTimeLine s
LEFT JOIN SnsMessage_tmp3 m ON m.feed_id = s.tid
LIMIT 20;
```

`feed_id` 和 `tid` 都可能为负，使用普通整数即可。

## 5. 其他表

### 5.1 SnsMainTimeLineBreakFlag

```sql
CREATE TABLE SnsMainTimeLineBreakFlag(
    tid INTEGER,
    tid_heigh_bit INTEGER,
    tid_low_bit INTEGER,
    break_flag INTEGER,
    CONSTRAINT _heigh_low_bit_tid PRIMARY KEY(tid_heigh_bit DESC, tid_low_bit DESC)
);
```

主时间线分页/断点信息。

### 5.2 SnsUserTimeLineBreakFlagV2

```sql
CREATE TABLE SnsUserTimeLineBreakFlagV2(
    tid INTEGER,
    tid_heigh_bit INTEGER,
    tid_low_bit INTEGER,
    break_flag INTEGER,
    user_name TEXT
);
```

按用户的朋友圈断点。

### 5.3 SnsTopItem_1

```sql
CREATE TABLE SnsTopItem_1(
    tid INTEGER,
    username TEXT,
    summary TEXT,
    create_time INTEGER,
    last_read_time INTEGER,
    is_read INTEGER
);
```

朋友圈顶部/未读/置顶条目。

### 5.4 SnsAdTimeLine

```sql
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
```

广告专用；`ad_content`/`extra_data` 可能是 JSON/XML。

### 5.5 SnsDraft / SnsPublishTask

```sql
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
```

草稿和发布任务。

### 5.6 SnsNoteVoice

```sql
CREATE TABLE SnsNoteVoice(
    tid INTEGER,
    data_id TEXT,
    buff TEXT,
    CONSTRAINT _tid_data_id PRIMARY KEY(tid, data_id)
);
```

朋友圈语音笔记。

### 5.7 SnsErrorMessage

```sql
CREATE TABLE SnsErrorMessage(
    local_id INTEGER PRIMARY KEY AUTOINCREMENT,
    error_type INTEGER,
    creat_time INTEGER,
    tid INTEGER,
    packed_info_data TEXT
);
```

注意字段名是 `creat_time`，不是 `create_time`。

### 5.8 SnsPendingDraftDeletionTable

```sql
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

待删除草稿。

### 5.9 SnsIgnoredDataItem

```sql
CREATE TABLE SnsIgnoredDataItem(
    tid INTEGER PRIMARY KEY
);
```

忽略的朋友圈条目。

## 6. Python：读取朋友圈

```python
import sqlite3
from datetime import datetime
from pathlib import Path

db = Path(r"C:\Users\你\Downloads\db_decrypted\sns\sns.db")
conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)

sql = """
SELECT tid, user_name, content
FROM SnsTimeLine
ORDER BY tid DESC
LIMIT 10
"""

for tid, user, content in conn.execute(sql):
    print("TID", tid, "USER", user)
    print(content[:500])
```

## 7. Python：提取正文

朋友圈 `content` 是 XML，可以用正则或 XML 解析器：

```python
import html
import re

def xml_value(xml: str, tag: str) -> str:
    match = re.search(rf"<{tag}\b[^>]*>(.*?)</{tag}>", xml, re.S | re.I)
    if not match:
        return ""
    text = match.group(1).replace("<![CDATA[", "").replace("]]>", "")
    text = html.unescape(text)
    text = re.sub(r"<[^>]+>", " ", text)
    return text.strip()

for tid, user, content in conn.execute("SELECT tid,user_name,content FROM SnsTimeLine LIMIT 5"):
    print(tid, user)
    print("正文：", xml_value(content, "contentDesc"))
    print("时间：", xml_value(content, "createTime"))
```

## 8. Python：点赞和评论

```python
sql = """
SELECT tid, user_name, content
FROM SnsTimeLine
ORDER BY tid DESC
LIMIT 5
"""
for tid, user, content in conn.execute(sql):
    print("朋友圈", tid, user)
    # 点赞用户通常出现在 likeUserList 内
    print("点赞片段：", xml_value(content, "likeUserList")[:300])
    # 评论列表
    print("评论片段：", xml_value(content, "commentList")[:300])
```

注意：

- 点赞/评论可能同时存在于主帖 XML 和 `SnsMessage_tmp3`；
- 不要只统计一张表；
- 删除状态要看 `del_status`；
- 同一条朋友圈可能被分页/缓存多次，按 `tid` 去重。

## 9. Python：某人的朋友圈

```python
user = "wxid_xxx"
sql = """
SELECT tid, content
FROM SnsTimeLine
WHERE user_name = ?1
ORDER BY tid DESC
LIMIT 20
"""
for tid, content in conn.execute(sql, (user,)):
    print(tid, xml_value(content, "contentDesc"))
```

## 10. 时间

`SnsTimeLine` 的 `createTime` 通常在 XML 内，是 Unix 秒。

```python
from datetime import datetime

ts = int(xml_value(content, "createTime"))
print(datetime.fromtimestamp(ts))
```

`SnsMessage_tmp3.create_time` 也是 Unix 秒。

## 11. 常见问题

### 11.1 tid 是负数

正常，是 64 位有符号 ID。

### 11.2 点赞数对不上

- 主帖 XML 和互动表可能重复；
- 可能有删除/忽略；
- 需要按 `tid`/评论 ID 去重；
- 不同版本字段不同。

### 11.3 内容中有转义 XML

用 `html.unescape`，再处理内部标签。

### 11.4 找不到某条朋友圈

- 可能被删除；
- 可能是广告/置顶；
- 可能在 `SnsAdTimeLine` / `SnsTopItem_1`；
- 可能不在当前时间线范围。

### 11.5 “位置动态”

位置动态也是朋友圈的一种，`content` 里可能没有正文，只有位置/媒体节点，不要当成损坏。

## 12. 速查

```text
sns/sns.db
├── SnsTimeLine                    朋友圈主表
├── SnsMessage_tmp3                评论/点赞互动
├── SnsTopItem_1                   顶部/置顶
├── SnsAdTimeLine                  广告
├── SnsDraft / SnsPublishTask      草稿/发布
├── SnsNoteVoice                   语音笔记
├── Sns*BreakFlag                  分页断点
├── SnsIgnoredDataItem             忽略项
├── SnsErrorMessage                错误
└── SnsPendingDraftDeletionTable   待删除草稿
```

关键点：

1. `tid`/`feed_id` 可以是负数；
2. `content` 是 XML；
3. 点赞/评论要结合主帖和互动表；
4. 时间是 Unix 秒；
5. 删除/忽略状态要过滤。
