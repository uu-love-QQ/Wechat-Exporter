# Wechat-Exporter

微信 4.x 本地数据库解密、全文检索与多格式聊天记录导出独立工具（纯 Rust 跨平台实现）。
基于标准 SQLCipher 4 密码学算法与内存扫描技术直接解密微信本地存储，独立运行于 **Windows** 与 **Linux** 两端。
---

## 版本与变更

当前稳定版本：**1.0.1**。完整变更见 [CHANGELOG.md](CHANGELOG.md)。

## 一、双端构建产物

项目中已配置好交叉编译工具链并生成了独立 Release 二进制：

- **Windows 64 位原生二进制**：
  `target/x86_64-pc-windows-gnu/release/wechat-exporter.exe`
- **Linux 64 位原生二进制**：
  `target/release/wechat-exporter`

### 本地编译命令

```bash
# 1. 编译 Linux 原生版本
cargo build --release

# 2. 编译 Windows 原生版本 (通过已配置的 mingw-w64 工具链)
cargo build --target x86_64-pc-windows-gnu --release
```

---

## 二、自动数据库检索机制

针对微信 4.x 的存储架构，CLI 支持**全自动路径检索**。当不传递 `--source-database` 时，程序会自动在以下标准目录中扫描有效的微信 4.x 数据库并按最后活跃时间排序：

### 覆盖的检索路径
1. **Windows 路径**：
   - Windows 注册表查询：读取 `HKCU\Software\Tencent\WeChat\FileSavePath`（微信自定义或迁移路径）
   - 各磁盘根目录及一级子目录深搜（直接命中诸如 `D:\电脑管家迁移文件\xwechat_files`、`D:\WeChat Files` 等）
   - `%USERPROFILE%\Documents\xwechat_files`、`%USERPROFILE%\文档\xwechat_files`
   - `%USERPROFILE%\xwechat_files`、`%APPDATA%\Tencent\WeChat\xwechat_files`
2. **Linux 路径**：
   - `$HOME/xwechat_files`、`$HOME/Documents/xwechat_files`、`$HOME/文档/xwechat_files`
   - 官方/Flatpak 微信目录：`~/.var/app/com.tencent.WeChat/data/xwechat_files`
   - Wine / Deepin 环境路径
3. **环境变量扩展**：
   - 可通过设置环境变量 `XWECHAT_FILES_DIR` 或 `ALPHA_WECHAT_ROOT` 自定义搜索根目录。

### 自动检索相关命令
```bash
# 查看本机所有已检测到的微信 4.x 数据库与账号列表
wechat-exporter detect

# 自动选用最新活跃账号列出会话列表（无需输入路径）
wechat-exporter conversation --key <64位Hex密钥>
```

---

## 三、密钥自动获取与图片解密机制
### 1. 主通行码自动提取
未传入 `--key` 时，CLI 自动执行内存安全扫描：
1. 打开 `Weixin.exe`，通过 Toolhelp 模块快照定位 `Weixin.dll`；
2. 读取 `Weixin.dll`，按 Alpha 的机器码特征提取 32 字节内部密钥掩码：
   ```
   48 BA <imm64> ??? 48 BA <imm64> ??? 48 BA <imm64> ??? 48 BA <imm64> ... 48 85 C0
   ```
   四段 `movabs rdx, imm64` 的立即数拼接为 32 字节掩码；
3. 只枚举 `Weixin.exe` 的 `MEM_COMMIT | MEM_PRIVATE` 私有堆页；
4. 以 32 字节滑动窗口识别 MSVC `std::string` 对象：
   `_Mysize == 32`、`_Myres == 47`、未使用区为零，并读取其指向的 32 字节内容；
5. 将候选内容与每个 `Weixin.dll` 掩码异或，使用数据库第一页的 16 字节 salt
   执行 `PBKDF2-HMAC-SHA512(passphrase, salt, 256000)`，再以 Page 1 的
   HMAC-SHA512 自校验，命中的 32 字节即为主通行码；
6. 主通行码在本次进程内缓存，后续所有分库直接派生密钥，不再重复扫描。

若上述路径未命中，再依次回退到 `x'<key><salt>'`、`Config.Cipher` 与
Salt 邻接二进制扫描；`--key` / stdin 仅作为显式覆盖手段。

### 2. 图片 `.dat` 全自动解密
微信 4.x 的图片存储在 `msg/attach/<md5>/<YYYY-MM>/Img/` 下，为加密 `.dat` 文件：
- **旧版格式**：单字节异或（CLI 自动通过图片头魔数推导 XOR 密钥解密为 JPEG/PNG/GIF/WEBP）；
- **V1 格式**（`07 08 56 31 08 07`）：AES-128-ECB + 0x88 异或（内置固定密钥解密）；
- **V2 格式**（`07 08 56 32 08 07`）：AES-128-ECB 动态密钥。
  - **自动从微信内存提取 AES Key**：在导出媒体时，CLI 自动提取任一 V2 密文样本，在 `Weixin.exe` 进程内存中精准匹配并验证 16 字节 AES 密钥；
  - **动态推导 XOR Key**：自动从文件末尾（JPEG `FF D9` 特征）计算 XOR 偏移；
  - 导出后自动保存为真实图片扩展名（`.jpg` / `.png` / `.webp` / `.gif`）。

---

## 四、常用命令示例

```powershell
# 1. 仅检测并列出本机所有微信数据库账号
.\wechat-exporter.exe detect

# 2. 列出最近活跃会话（自动关联电脑管家迁移路径，传入主口令）
.\wechat-exporter.exe conversation --key <64位Hex主通行码>

# 3. 全文检索关键词
.\wechat-exporter.exe search "合同" --key <64位Hex主通行码> --ascending

# 4. 导出指定会话（含图片自动解密为真实图片格式）：
.\wechat-exporter.exe conversation --export `
  --key <64位Hex主通行码> `
  --conversation-id "wxid_85da986onp4h22" `
  --format all `
  --include-media `
  --output-dir "C:\Users\你\Downloads\uuMovies\test"

# 5. 全自动模式（微信登录在线时，无需提供路径与 key）：
.\wechat-exporter.exe conversation --export `
  --conversation-id "wxid_85da986onp4h22" `
  --format all `
  --include-media `
  --output-dir "C:\Users\你\Downloads\uuMovies\test"
```

## 五、子命令与参数

根命令只显示子命令；每个子命令只显示自己会用到的参数。例如
`wechat-exporter conversation --help` 同时列出列表与导出参数；
`conversation` 缺省列出会话，只有加 `--export` 才执行导出。完整帮助优先级：

```bash
wechat-exporter --help            # 子命令列表
wechat-exporter conversation --help # 列表 + 导出参数
```

### init

初始化本地配置，提取主密钥、图片 AES/XOR 密钥并写入配置文件，后续可免扫描运行。

```bash
wechat-exporter init --key <64位Hex密钥>
wechat-exporter init --source-database /path/to/db_storage --image-key <32位Hex>
```

| 参数 | 说明 |
|---|---|
| `-s, --source-database <PATH>` | 指定 `db_storage` 或会话数据库 |
| `-K, --key <HEX>` | 64 位 Hex 数据库密钥 |
| `--key-file <PATH>` | 从文件读取数据库密钥 |
| `--image-key <HEX>` | V2 `.dat` 图片的 16 字节 AES 密钥 |
| `--self-username <WXID>` | 指定当前登录账号 |
| `--plaintext` | 按未加密 SQLite 打开 |
| `--debug` | 详细调试输出 |

### detect

只检索微信 4.x 数据库位置与账号，不解密、不读取密钥。

```bash
wechat-exporter detect
wechat-exporter detect --source-database /path/to/xwechat_files
```

| 参数 | 说明 |
|---|---|
| `-s, --source-database <PATH>` | 只扫描指定目录；缺省扫描系统标准路径 |
| `--debug` | 详细调试输出 |

### contact

```bash
wechat-exporter contact --query 张 --kind friend --page-size 50
```

列出联系人和群聊，支持：

- `--query/-q`：按 WXID、昵称、备注、别名过滤；
- `--kind`：
  - `all`：全部；
  - `friend`：好友（含企业好友）；
  - `group`：群聊（含企业群聊）；
  - `official`：公众号；
  - `service`：服务号；
  - `enterprise`：企业好友；
  - `enterprise-group`：企业群聊；
  - `former`：曾经的好友；
  - `blocked`：黑名单；
- `--page-size/-p`：每页数量，1–1000；
- `--cursor`：上一页返回的 `next_cursor`；
- 输出包含 `id`、昵称、备注、别名、显示名、类型、群聊/企业/黑名单/曾经好友标记、公众号类型、头像 URL。

### search

跨会话全文检索，支持 zstd 压缩文本。

```bash
wechat-exporter search "合同" --key <64位Hex密钥> --ascending
wechat-exporter search "关键词" --source-database /path/to/db_storage --page-size 100
```

| 参数 | 说明 |
|---|---|
| `TEXT` | 检索词（位置参数，最多 512 字节） |
| `-s, --source-database <PATH>` | 指定数据库路径；缺省自动检索 |
| `-K, --key <HEX>` / `--key-file <PATH>` | 数据库密钥 |
| `--ascending` | 按时间正序；缺省倒序 |
| `-p, --page-size <N>` | 单页数量 1..1000，缺省 50 |
| `--cursor <BASE64>` | 上一页返回的 `next_cursor` |
| `--self-username <WXID>` | 指定当前登录账号 |
| `--plaintext` | 按未加密 SQLite 打开 |
| `--debug` | 详细调试输出 |

### conversation

缺省列出会话；加 `--export` 导出指定会话，`--format all` 生成全部 11 种格式。

```bash
wechat-exporter conversation --key <64位Hex密钥> --page-size 20
wechat-exporter conversation --export --key <64位Hex密钥> \
  --conversation-id "123456789@chatroom" --format html --output-dir /path/to/out
wechat-exporter conversation --export --source-database /path/to/db_storage \
  --conversation-id wxid_xxx --format all --include-media --output-dir /path/to/out
wechat-exporter conversation --export --key <64位Hex密钥> \
  --conversation-id wxid_xxx --format json --report --output-dir /path/to/out
```

`--report` 额外生成 `conversation.report.html`：蓝紫渐变头图与 6 项核心指标
（消息数、总字数、活跃天数、单日最高、平均字数、互动人数）、对话比例 PK 条、
活跃度概览、互动排行、我/其他人的每日趋势、24 小时与星期分布、消息类型环形图、
最活跃日期 TOP 10（含双方占比）、以及双方高频词云。图表附带数据表和
`aria-label` 摘要；报告为自包含静态 HTML，无外部脚本和网络请求，支持亮/暗主题。

| 参数 | 说明 |
|---|---|
| `-s, --source-database <PATH>` | 指定数据库路径；缺省自动检索 |
| `-K, --key <HEX>` / `--key-file <PATH>` | 数据库密钥 |
| `--self-username <WXID>` | 指定当前登录账号 |
| `--plaintext` | 按未加密 SQLite 打开 |
| `--export` | 执行导出；缺省只列出会话 |
| `-q, --query <TEXT>` | 按会话名称、备注、昵称、别名或 ID 过滤 |
| `-p, --page-size <N>` | 列表单页数量 1..1000，缺省 50 |
| `--cursor <BASE64>` | 列表上一页返回的 `next_cursor` |
| `--image-key <HEX>` | V2 `.dat` 图片 AES 密钥；缺省自动提取 |
| `-c, --conversation-id <ID>` | 会话 ID，可重复指定 |
| `-f, --format <NAME>` | 格式或 `all`，可重复指定 |
| `-m, --include-media` | 提取媒体到 `media/` |
| `--start-time-ms <MS>` / `--end-time-ms <MS>` | 时间范围（Unix 毫秒） |
| `-o, --output-dir <DIR>` | 绝对路径输出目录；不存在时自动创建 |
| `-k, --keep-output` | 保留临时输出目录 |
| `--report` | 生成可视化分析报告 `conversation.report.html` |
| `--upload` | 导出完成后打包为 ZIP 上传到 WebDAV |
| `--upload-part-size <MB>` | 上传分卷大小，缺省 100 MB；`0` 表示不分卷 |
| `--backup` | 生成 `backup.zip`：`backup.db` + HTML + 图片/语音/表情等媒体，配合 `recovery` 写回微信数据库 |
| `--silent` | 不打印进度条，仅保留最终结果/错误 |
| `--debug` | 详细调试输出 |

### favorite

读取 `favorite/favorite.db` 的收藏列表，不依赖聊天数据库。

```bash
wechat-exporter favorite
wechat-exporter favorite --type article
wechat-exporter favorite -q 关键词 --limit 20 --offset 0
wechat-exporter favorite --full
```

- `--type`：`all` / `text` / `image` / `voice` / `video` / `article` / `file` / `chat-record` / `location` / `note` / `mini-program` / `card` / `video-channel` / `other`；
- `-q, --query`：搜索收藏内容、来源或聊天名；
- `--limit/-n`：1–1000，缺省 50；
- `--offset`：分页偏移；
- `--full`：输出完整内容，而不只是预览；
- `-o, --output-dir`：导出收藏清单、每条原始 XML 和聊天记录 JSON；
- 文件需要放在 `db_storage/favorite/favorite.db`，支持配置中的路径与密钥。

输出包含收藏 ID、类型、时间、标题/描述、来源、聊天名；文章、语音、文件等类型还会输出 URL、时长、大小，聊天记录会解析出消息数组。

导出示例：

```bash
wechat-exporter favorite --type voice -o /path/to/out
wechat-exporter favorite --type chat-record -o /path/to/out
```

输出目录下会生成：

- `favorites.json`：完整清单；
- `favorites/<id>-<类型>.xml`：原始收藏 XML；
- `favorites/<id>-聊天记录.json`：解析后的聊天记录消息。

### report

生成独立分析报告（HTML，自包含、离线）。所有 report 子命令都需要绝对路径
`--output-dir`；数据库与密钥参数与 conversation 相同。

```bash
# 私聊分析：扫描所有私聊，统计好友消息量、发送比例、类型、时段、月度趋势
wechat-exporter report private --key <64位Hex> --output-dir /path/to/report

# 群聊分析：群成员明细、发言排行、活跃时段、媒体内容
wechat-exporter report group --key <64位Hex> \
  --conversation-id "123456789@chatroom" --output-dir /path/to/report

# 双人报告：指定好友的双方聊天分析，可限定年份
wechat-exporter report dual --key <64位Hex> \
  --conversation-id wxid_xxx --year 2024 --output-dir /path/to/report

# 年度报告：指定年份；--year 0 生成跨年度长期报告
wechat-exporter report annual --key <64位Hex> --year 2024 --output-dir /path/to/report

# 年度报告同时导出 PNG（需要 Chrome/Edge/Chromium）
wechat-exporter report annual --key <64位Hex> --year 2024 \
  --output-dir /path/to/report --png

# 直接从已导出的 JSON 生成（无需数据库和密钥）
wechat-exporter report private --from-json /path/to/export --output-dir /path/to/report
wechat-exporter report dual --from-json /path/to/export/Alice/conversation.json \
  --output-dir /path/to/report
```

| 类型 | 输出 | 说明 |
|---|---|---|
| `private` | `private-report.html` | 好友消息排行、发送比例、消息类型、24 小时/月度趋势、媒体 |
| `group` | `group-report.html` | 群成员名册/明细、发言排行、活跃时段、媒体 |
| `dual` | `dual-report.html` | 双方对话比例、趋势、时段、类型、高频词 |
| `annual` | `annual-report.html` | 年度/长期汇总、月度趋势、好友与群聊排行、最活跃日期 |
| `annual --png` | `annual-report.html` + `annual-report.png` | 额外生成完整年度报告长图 |

| 参数 | 说明 |
|---|---|
| `-o, --output-dir <DIR>` | 报告输出目录（必填，绝对路径） |
| `-c, --conversation-id <ID>` | `group` / `dual` 必填 |
| `--year <YYYY>` | `group` / `dual` / `annual` 的年份；缺省或 `0` 为全部历史 |
| `--from-json <PATH>` | 从 `conversation.json`、`export-report.json` 或导出目录读取，跳过数据库 |
| `--png` | 仅年度报告：额外生成 `annual-report.png`，需要本机 Chrome/Edge/Chromium |
| `--browser <PATH>` | 指定 Chrome/Edge/Chromium 可执行文件，供 `--png` 使用 |
| `--silent` | 不打印进度条 |
| `-s, --source-database` / `-K, --key` / `--key-file` | 与其他子命令相同 |

`group` 报告优先从联系人库的 `chatroom_member` + `name2id` 读取完整群名册；
表结构不兼容时回退为“消息中出现的成员”，并在报告中标注成员数。

导出与报告执行时会在终端显示进度条：消息 `已处理/总数`、附件 `已处理/总数`，
报告还会显示当前扫描会话与阶段。`--silent` 可关闭进度条，保留最终 JSON/错误输出。

`--png` 使用无头 Chrome/Edge/Chromium 渲染自包含 HTML，并把截图裁剪到实际内容高度；
默认宽度 1600px。浏览器自动查找，找不到时可用 `--browser` 指定路径；也可以用环境变量
`WECHAT_EXPORTER_BROWSER`。

导出格式、媒体附件、报告会话读取和 WebDAV 文件上传使用并行任务；
SQLite 读取按 worker 独立打开只读连接，媒体落盘使用内容寻址并加写锁。

`conversation` / `search` 返回的 `next_cursor` 是分页游标（Base64 编码的偏移量），
不是密钥或错误码；把上一页返回的值原样传给 `--cursor` 即可继续下一页。

`export-report.json` 的顶层和每个 `conversations[]` 条目都包含
`start_time` / `end_time`（Unix 秒）与 `start_time_text` / `end_time_text`，
可直接用于确认导出范围，也可作为 `report --from-json` 的输入。

### moments

```bash
wechat-exporter moments --year 2026 --output-dir /path/to/output
```

分析朋友圈并输出 `moments.json`：

- `posts`：本人发布的朋友圈数量；
- `postTypes`：图片、文字、链接、视频等类型分布；
- `likesReceived` / `likesGiven`：收到和给出的赞；
- `topLikers`：给你点赞最多的好友（最多 10 位）；
- `topLikedAuthors`：你点赞最多的好友（最多 10 位）。

不传 `--output-dir` 时直接打印 JSON；`--year 0` 或不传表示全部历史。

时间线：

```bash
wechat-exporter moments --year 2026 --posts --limit 50 --offset 0
```

- `--posts`：在 JSON 中增加 `timeline` 数组与 `timelineTotal`；
- `--limit` / `--offset`：分页；
- `--user`：按作者 WXID 或昵称过滤；
- 每条时间线包含作者、时间、正文、媒体 URL、点赞用户和头像 URL。

### decrypt

把 `db_storage` 下的 SQLCipher 数据库逐库转为普通 SQLite，目录结构保持不变：

```bash
wechat-exporter decrypt --key <64位Hex密钥> \
  --source-database /path/to/db_storage --output-dir /path/to/decrypted
```

- 输出为可直接用 `sqlite3` / DB Browser 打开的标准 SQLite 文件；
- 源数据库不会被修改，转换在临时副本上完成；
- 目标文件已存在时默认失败，加 `--force` 覆盖；
- 只处理 `.db` 文件，并保留 `contact/`、`message/`、`session/` 等相对路径；
- 交互终端下显示逐库进度；`--silent` 可关闭进度，保留最终 JSON。

### recovery

恢复数据。`--backup` 导出只保存选中的会话或时间范围，`recovery` 只恢复备份中存在
的行，不会清空目标库中的其他聊天记录。

#### 1. 按会话备份原始 WCDB 行（推荐）

```bash
# 只备份某个会话；自动生成 backup.zip（内部含 backup.db、conversation.html、media/）
wechat-exporter conversation --export --backup \
  --key <64位Hex密钥> --conversation-id wxid_xxx \
  --output-dir /path/to/backup

# 关闭微信后，从压缩包把该会话缺失的消息写回原始加密库
wechat-exporter recovery --input /path/to/backup/backup.zip \
  --target-database /path/to/db_storage --key <64位Hex密钥> --force
```

压缩包内容：

- `backup.db`：`wcdb_meta`、`wcdb_messages`、`wcdb_name2id`，保存 `local_id`、
  `server_id`、`local_type`、`sort_seq`、`real_sender_id`、`message_content`、
  `compress_content`、`packed_info_data`、`WCDB_CT_*` 等原始字段；
- `conversation.html`：可直接打开的离线 HTML 记录；
- `media/`：图片、语音、表情、视频、文件等已解密媒体；
- `manifest.json`：会话、消息数、媒体统计。

`--backup` 只生成压缩包，不会修改普通 `--format sqlite` 的 `conversation.sqlite`
导出。可重复传入 `-c` 或使用 `--start-time-ms` / `--end-time-ms` 只备份部分消息；
多个会话会分别生成 `backup-<会话>.zip`。

恢复行为：

- 从 zip 中读取 `backup.db`，按 `local_id` / `server_id` 只插入目标中不存在的消息；
- 保留其他会话和已有新消息；目标消息表不存在时使用备份建表 SQL 创建；
- 恢复前自动备份目标数据库，返回 JSON 包含 `inserted` / `skipped` 数量；
- `--force` 是写回微信数据库的必需确认开关；建议先退出微信。

#### 2. 完整数据库备份恢复

```bash
wechat-exporter decrypt --key <64位Hex密钥> \
  --source-database /path/to/db_storage --output-dir /path/to/backup

wechat-exporter recovery --input /path/to/backup \
  --target-database /path/to/db_storage --key <64位Hex密钥> --force
```

- 适用于完整迁移/灾难恢复，会逐表覆盖目标数据库内容并保留原有 salt/key；
- `--input`/`--backup` 对应 `decrypt` 生成的含 `.db` 的目录；
- 恢复前备份目标 `.db` 及 WAL/SHM 到 `db_storage` 同级隐藏目录。

#### 3. 离线恢复会话备份

不带 `--target-database` 时，`recovery` 只把备份转成 JSON/HTML 等格式，不写入微信：

```bash
wechat-exporter recovery --input /path/to/backup \
  --output-dir /path/to/recovered --format json --format html
```

- 默认输出 `conversation.json`，可用 `-f all` 恢复全部格式；
- 加 `--include-media` 可复制备份中的 `media/`；
- 不修改任何微信数据库。

### backup

配置 WebDAV 后，`conversation --export --upload` 会把整个输出目录打包成一个
ZIP，再上传到 `<url>/<remote_path>/<输出目录名>/`。默认按 **100 MB** 分卷，
适合有单文件大小限制的 WebDAV。

```bash
# 保存 WebDAV 配置（当前目录 .wechat-exporter-backup.json）
wechat-exporter backup webdav \
  --url https://dav.example.com/remote.php/dav/files/user \
  --username <user> --password <pass> --remote-path wechat-exports

wechat-exporter backup show    # 查看配置，密码隐藏
wechat-exporter backup test    # PROPFIND 测试连接
wechat-exporter backup clear   # 删除配置

# 导出后自动打包并上传；默认 100 MB 分卷
wechat-exporter conversation --export --key <64位Hex> \
  --conversation-id 123456789@chatroom --format all --include-media \
  --output-dir /path/to/out --upload

# 指定分卷大小，例如 20 MB；0 表示不分卷
wechat-exporter conversation --export --key <64位Hex> \
  --conversation-id 123456789@chatroom --format all --include-media \
  --output-dir /path/to/out --upload --upload-part-size 20
```

上传内容：

```text
out.zip                       # 不超过分卷大小时
out.zip.part001               # 超过分卷大小时
out.zip.part002
out.zip.parts.json            # 分卷清单，含顺序和大小
```

本地的 `out/` 目录不会删除；远端目录不存在时自动 `MKCOL` 创建，分卷并发上传。
密码保存在本地配置文件中（Unix 下权限 0600）。

合并分卷（Linux/macOS）：

```bash
cat out.zip.part* > out.zip
unzip out.zip
```

Windows PowerShell：

```powershell
Get-Content .\out.zip.part* -AsByteStream | Set-Content .\out.zip -AsByteStream
```

必须按 `part001`、`part002` 的顺序拼接；`out.zip.parts.json` 可用于校验顺序和大小。

### upgrade

```bash
# 下载并安装最新版
wechat-exporter upgrade

# 指定版本
wechat-exporter upgrade --version v0.12.1

# 国内网络前缀（拼接到 GitHub 下载地址前）
wechat-exporter upgrade --prefix https://ghfast.top/
```

Windows 下会先下载新版，退出后由后台命令替换并重启；Linux 直接替换当前二进制。
`--prefix` 只是 URL 前缀，不做域名固定；不检查 release 中是否存在其他附件。

## WCDB 数据结构

微信 4.x `db_storage` 的数据库清单、表结构、字段作用、zstd/加密/WAL/FTS
等特殊处理，详见 [docs/WCDB.md](docs/WCDB.md)。

文档包含零基础教程和可直接运行的 Python 示例：

```bash
python docs/examples/inspect_wcdb.py /path/to/decrypted
python docs/examples/read_messages.py /path/to/decrypted wxid_xxx --limit 20
```

示例依赖和完整说明见 [docs/examples/README.md](docs/examples/README.md)。

按数据库拆分的详细文档：

- [docs/databases/README.md](docs/databases/README.md)
- [session](docs/databases/01-session.md)
- [contact](docs/databases/02-contact.md)
- [message](docs/databases/03-message.md)
- [media](docs/databases/04-media.md)
- [favorite](docs/databases/05-favorite.md)
- [sns](docs/databases/06-sns.md)
- [emoticon](docs/databases/07-emoticon.md)
- [hardlink](docs/databases/08-hardlink.md)
- [head_image](docs/databases/09-head-image.md)
- [bizchat/chatbot](docs/databases/10-bizchat-chatbot.md)
- [general](docs/databases/11-general.md)
- [message_resource](docs/databases/12-message-resource.md)
- [solitaire/third_app_icon](docs/databases/13-solitaire-third-app-icon.md)
- [FTS](docs/databases/14-fts.md)
- [内部表](docs/databases/15-internal-tables.md)

## 六、HTML 界面与引用跳转

HTML 导出由 `web/` 前端模板生成（Vite + React 19 + Tailwind v4 + shadcn/ui），
构建为单文件 HTML 后由 Rust 注入 JSON。修改 UI 后重新生成模板：

```bash
cd web
npm install
npm run export:template   # 构建聊天模板与 shadcn 报告模板
```

- 聊天界面 → `src/export/chat_template.html`
- 报告界面（React + shadcn/ui）→ `src/export/report_template.html`

报告使用 shadcn 的 Card、Table、Tabs、Progress、Badge、Separator 组件，
由 Rust 注入 `report-data` JSON；报告模板独立构建为单文件
`dist-report/report.html`，不依赖外部脚本或 CDN。

HTML 导出是自包含的响应式聊天界面：吸顶标题栏、即时搜索（`/` 聚焦、`Esc` 清空）、
亮/暗主题切换、按日期分组、左右消息气泡、图片/音频/视频内嵌、缺失附件提示与“回到最新”按钮。
配色使用 Messenger 蓝 + 柔和蓝灰表面，亮/暗两套 token 配对；左上角会话头像优先读取联系人
联系人头像 URL（`small_head_url` / `head_img_url` 等）并缓存到 `media/avatar/`，失败则回退首字母色块；语音消息按 XML
`voicelength` 显示时长（如 `0.5 秒`、`2 分 05 秒`），并使用自定义语音条
（播放/暂停、波形进度、点击跳转播放位置），不渲染浏览器原生 audio 控件。
群聊消息显示彩色发送者头像；私聊只在左上角保留会话头像，消息列表不重复头像。
图片/表情点击后在 Dialog 中预览大图，不再新开标签页；本地已索引到的图片/表情不再显示
`[图片]`/`[动画表情]` 占位文字。合并转发的聊天记录（type 19，解析 `<recorditem>`）显示
为可点击卡片，弹窗内按发送者/时间列出嵌套消息明细。

每条消息写入稳定锚点 `id="msg-N"`。引用消息（type 57）解析 `<refermsg><svrid>`：
在导出集合内找到原消息时提供 `#msg-N` 跳转；找不到时仍优先展示原发送者与原内容，
仅以弱提示标注「原消息不在导出范围」，不生成悬空链接。回复正文独立渲染。

可访问性与性能：移动端触控目标 ≥44px、搜索框 48px；`/` 聚焦、`Esc` 清空；
跳转目标有可见 focus 轮廓；结果计数使用 `aria-live`；尊重 `prefers-reduced-motion`；
图片声明宽高降低 CLS；消息行使用 `content-visibility: auto` 优化长会话。

### 语音库探测开销

- 语音库只探测 `media_*.db` / `biz_media_*.db` / `voice*.db` 等专用媒体库；
  仅当这些库都没有 `voice_data` 列时才回退扫描 `message_*.db` / `biz_message_*.db` 分片，
  避免每次展开数百张 `Msg_*` 表。
- `--debug` 的探测输出同时过滤掉非语音表（`Name2Id`、`TimeStamp` 等）。
- 媒体索引每次导出都会重新遍历，确保刚收到的新图片、新附件能立即命中，不使用缓存。

## 私有文件（novac2c）

微信 4.x 的私有文件（PDF/ZIP/APK 等）只给出 `cdnattachurl` + `aeskey`。该 CDN 端点需要
微信客户端专有鉴权；本工具不实现，也不会向该端点发起请求。

导出时只使用本地媒体索引（`msg/attach`、`msg/file`、缓存等）。本地未索引到的私有文件
在消息中保留为空缺（不写 `media.path`），导出报告将其计入缺失，不生成占位文件。


### 媒体源优先级（对齐 Alpha 实测）

| 消息类型 | 原图字段 | 说明 |
|---|---|---|
| 表情 `<emoji>` | `cdnurl` | **明文原图**，字节数与 XML `len` 相等，MD5 等于消息 `md5` |
|  | `encrypturl` | AES 加密变体 |
|  | `externurl` | AES 加密的 wxgf 预览（体积最小） |
| 图片 `<img>` | `cdnbigimgurl` | 原图 |
|  | `cdnmidimgurl` | 中图 |
|  | `cdnthumburl` | 缩略图 |
| 私有文件 `<appattach>` | `cdnattachurl` | 仅本地索引；缺失时留空 |

实测（消息 17/21/52/884）：从 `cdnurl` 拉取的字节与 Alpha 导出的文件**逐字节相同**，
大小分别等于 XML 声明的 29560 / 99666 / 20563 / 90546。导出时若本地明文比声明长度小 1% 以上
即判定为预览，并在写盘前用 CDN 原图替换，避免留下缩略图或二次压缩图。

## 语音消息

微信 4.x 的语音不在消息库里，而在 `db_storage/media/*.db`（或消息分片）的
`voice_data` 列中，按 `server_id` / `local_id` / 聊天+时间定位。导出流程：

1. 启动时扫描 `db_storage` 及其 `message`、`message/media`、`message/voice`、`voice` 子目录中的数据库，
   自动识别明文/加密库，并按表名、列名或首条 BLOB 的音频魔数（RIFF/#!SILK/#!AMR）定位语音表；
2. `chat_name_id` 按 `Name2Id.rowid` 映射；查询依次尝试
   `svr_id`（全局唯一，实测与 Alpha 导出逐字节一致）→ `msg_id` → 聊天+时间窗；
   时间窗同时兼容秒与毫秒（`create_time` 单位为毫秒），窗口 ±300s 内取
   `abs(create_time-消息时间)` 最近的一行，与 Alpha 的
   `chat_name_id IN (SELECT rowid FROM Name2Id …) AND create_time BETWEEN …` 语义一致。
   **WeChat 4.x 的 `VoiceInfo.local_id` 是媒体行号，不是消息的 `local_id`**，跨会话大量重复，
   因此绝不作为查询键（v0.1.3 曾直查该列，导致 7 条语音串到其他会话）；
3. `RIFF` 开头直接按 `.wav` 保存；
   `#!SILK`（含腾讯前缀 `\x02#!SILK_V3` 及 H3/L3/VH3/VL3 变体）用内置 `silk-codec` 解码为
   24 kHz / 单声道 / 16-bit PCM WAV，与 Alpha 导出的 WAV 规格一致，不依赖外部 ffmpeg；
   `#!AMR` 先尝试 ffmpeg 转码，失败则保留 `.amr`；无法识别的 BLOB 以 `.bin` 落盘而不是丢弃。

`--debug` 会打印 `[voice] 命中 …: N 字节, 头部 …`、`[voice] SILK 解码成功: … -> WAV 24000Hz`
以及 `• 语音库命中: N 字节 -> media/voice/<sha256>.wav`；未命中时打印 `chat_rowid`、各键命中行数
和按时间最近的三行样本，便于定位库内 id/时间单位。

