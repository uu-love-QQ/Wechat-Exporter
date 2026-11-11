# Reverse-engineering notes

## 1. Installer

`Alpha-0.3.5-x64-Setup.exe` is a **Nullsoft Scriptable Install System 3
(Unicode)** self-extracting archive, PE32 GUI, 132 MiB.

```text
7z l Alpha-0.3.5-x64-Setup.exe
  Type = Nsis
  SubType = NSIS-3 Unicode
  $PLUGINSDIR/app-64.7z          137,510,945 bytes
  $PLUGINSDIR/{System,UAC,StdUtils,nsDialogs,nsexec,nsis7z,WinShell}.dll
  $R0/Uninstall Alpha.exe
```

`app-64.7z` expands to an Electron 30-era application (`Alpha.exe`, `resources/app.asar`,
Chromium `.pak` files, `ffmpeg.dll`, `dxil.dll`, etc.) and a `resources/bin`
directory containing native Rust gateways and the data engine.

`resources/app.asar` (`@alpha/desktop` 0.3.5) embeds the main process
(`out/main/index.js`, 3.1 MB) and a React renderer. The main process is a thin
IPC/protobuf client; the actual WeChat work happens in native components.

## 2. Component graph

`resources/component-manifest.json` and `resources/bin/peer-manifest.json`
identify the shipped components:

| Binary | Size | Role |
|---|---:|---|
| `alpha-engine.exe` | 21.8 MB | engine / RPC dispatcher |
| `alpha-server.exe` | 29.4 MB | gateway |
| `alpha-mcp-stdio.exe` | 17.1 MB | MCP gateway |
| `alpha-data-host.exe` | 15.9 MB | WCDB data host |
| `alpha-data-analysis.exe` | 1.5 MB | analysis worker |
| `alpha-security-host.exe` | 10.9 MB | component broker / license |
| `alpha-key-worker.exe` | 2.1 MB | XKey worker |
| `alpha-cli.exe` | 1.7 MB | chat-export CLI |
| `alpha-message-send.exe` | 3.3 MB | controlled writer |
| `alpha-official-export.exe` | 3.5 MB | official-account export |
| `alpha-schedules.exe` | 4.0 MB | scheduled export |
| `alpha-chatlab-pull.exe` | 1.5 MB | ChatLab integration |

`resources/native/wcdb/alpha_native.dll` contains the WCDB queries and schema
probes; `alpha_sqlcipher_readonly.dll` bundles SQLCipher; the manifest declares
capabilities `live_readonly_sql`, `sqlcipher-keyed-open`, `manual_database_key`,
`automatic_key_acquisition`, `retention_writer`, and Ed25519-signed component
delivery.

## 3. The CLI contract

The CLI usage string and option validation are recoverable from
`resources/bin/alpha-cli.exe`:

```text
Reads a local WeChat WCDB database through Alpha's read-only Engine path and
exports JSON, Markdown, ChatLab JSON/JSONL, HTML, TXT, XLSX, Arkme JSON, SQL,
and WeClone CSV. The 32-byte database key is read only from stdin, or acquired
through Alpha XKey with --xkey-process-id.

--source-database <session.db|db_storage>
--native-library / --wcdb-library / --xkey-process-id
--list-conversations | --search <text> [--ascending]
--page-size <1..1000> --cursor <base64>
--include-media --format <name|all>... --conversation-id <id>
--start-time-ms <positive-ms> --end-time-ms <positive-ms>
--keep-output --output-dir <absolute-dir>
```

> 以上是原 Alpha 二进制的参数契约。本仓库的 `wechat-exporter` 只保留数据库、
> 查询、导出和调试参数；兼容类与进程注入类旧参数已移除。

Recovered output file names:

```text
json          conversation.json
markdown      conversation.md
chatlab-json  conversation.chatlab.json
chatlab-jsonl conversation.chatlab.jsonl
html          conversation.html
txt           conversation.txt
xlsx          conversation.xlsx
arkme-json    conversation.arkme.json
sql           conversation.sql
sqlite        conversation.sqlite
weclone       conversation.weclone.csv
```

The binary validates: the key is exactly 64 hex characters, export time bounds
are positive int64 milliseconds, page size is 1..1000, conversation IDs are
nonempty, ≤1024 bytes and control-free, list/search reject export options,
`--ascending` requires `--search`, and `--format`/bounds/`--conversation-id`
require `--output-dir` or `--keep-output`. Output directories must be absolute,
must have an existing parent, and must not exist.

## 4. WeChat 4.x data model

The data host exposes JSON contracts named `ALPHA_WCDB_MESSAGES_V*`,
`ALPHA_WCDB_SESSIONS_V1`, `ALPHA_WCDB_CONTACTS_V1`, `ALPHA_WCDB_SEARCH_V2`, and
friends. Static SQL strings in `alpha_native.dll` plus the live schema probes
recover this layout:

```text
<account>/db_storage/
├── session/session.db
│   └── SessionTable(username, type, unread_count, sort_timestamp,
│                     last_timestamp, last_msg_type, last_msg_sub_type,
│                     last_msg_sender, last_sender_display_name, ...)
├── contact/contact.db
│   └── contact(username, alias, remark, nick_name, local_type, flag,
│                delete_flag, ...)
│   └── chatroom_member(room_id, member_id)
├── message/message_N.db
│   ├── Name2Id(user_name, is_group)          rowid -> wxid
│   └── Msg_<md5(username)>                   one table per conversation
└── message/biz_message_N.db                  same Msg_ row shape
```

`Msg_<md5>` columns observed in the engine:

```text
local_id, server_id, local_type, sort_seq, real_sender_id, create_time,
status, upload_status, download_status, server_seq, origin_source, source,
message_content, compress_content, packed_info_data,
WCDB_CT_message_content, WCDB_CT_source
```

`local_type` is packed: `base = local_type & 0xffffffff`,
`subtype = local_type >> 32`. Examples: `244813135921` is base 49 / subtype 57
(quote); `266287972401` is base 49 / subtype 62 (pat).

`WCDB_CT_message_content = 4` or a zstd magic prefix (`28 b5 2f fd`) means the
payload is zstd-compressed. Group text rows may start with `wxid:\n`; the
prefix is authoritative when historical shards disagree about
`real_sender_id`.

`Name2Id` is per message shard. The data host's SQL confirms it maps
`rowid=real_sender_id` to `username`, and uses `md5(username)` for the
`Msg_` table suffix.

## 5. Encryption

Each WCDB file is SQLCipher 4: AES-256-CBC, HMAC-SHA512, 4096-byte pages,
80-byte reserve (16-byte IV + 64-byte HMAC). The 64-hex key accepted by
`alpha-cli.exe` is a **raw 32-byte enc key**, applied as
`PRAGMA key = "x'<64hex>'"`; the per-file salt is read from the first 16 bytes.
This implementation uses rusqlite's bundled SQLCipher and falls back to a
read-write handle (with `query_only=ON`) for live WAL databases.

## 6. Media

Images live under
`<account>/msg/attach/<md5(conversation)>/<YYYY-MM>/Img/<md5>[_t|_h].dat`.
The CLI decrypts the three observed `.dat` formats:

- legacy single-byte XOR (key inferred from PNG/JPEG/GIF/WEBP/BMP header);
- V1 (`07 08 56 31 08 07`) with fixed AES-128-ECB key
  `cfcd208495d565ef` plus a 0x88 XOR tail;
- V2 (`07 08 56 32 08 07`) with an AES key extracted from the running WeChat
  process (`--image-key` here).

Videos are stored under `msg/video/<YYYY-MM>/<md5>.mp4`; other attachments are
resolved by file name or MD5 from the account media tree.

## 7. Implementation

`alpha-cli-rs` replaces the engine path with direct SQLCipher access:

```text
cli.rs              option parsing and Alpha-compatible validation
db.rs               SQLCipher open/read-only/WAL handling
wechat/discover.rs  account, db_storage, shards, session/contact discovery
wechat/shards.rs    Name2Id maps, Msg_ row loading, zstd, dedup, ordering
wechat/parse.rs     local_type and appmsg/XML content parsing
wechat/build.rs     sender, direction, display-name enrichment
wechat/media.rs     media index, .dat decryption, copying
decrypt.rs          SQLCipher -> plaintext SQLite export
recovery.rs         restore exports from conversation.sqlite backups
export/mod.rs       eleven format writers + report
```

The integration fixture (`src/bin/make_fixture.rs`) builds an encrypted
WeChat-shaped account with two shards, zstd content, app messages, a quote, a
system row, and media. `cargo test` runs unit tests for content/DAT handling and
five end-to-end CLI tests.

## 8. Not reproduced

XKey process-memory key acquisition, license lease/signature checks, CDN media
download, voice transcription (Silk→WAV→text), Moments/favorites/contacts
exports, avatar extraction from `head_image.db`, and the desktop UI/AI analysis
features. Those require Alpha services or Windows-only native components; the
local chat-export CLI remains fully functional without them.


## 微信 4.1+ 密钥解析实现参考 (wcdb-key-tool)

参考开源项目 [TANGandXUE/wcdb-key-tool](https://github.com/TANGandXUE/wcdb-key-tool)，微信 4.1+ 版本在运行时会将 WCDB 的 `Config.Cipher` 配置对象常驻内存：

1. **对象标记定位**：
   - 搜索常量字符串 `com.Tencent.WCDB.Config.Cipher` (30 字节) 的内存地址；
   - 构造 `(needle_addr: u64, 30: u64)` 引用指针组并在私有堆与内存块中寻找引用节点；
2. **异或解密与密钥提取**：
   - 定位配置节点结构体偏移 `0x28` 的配置指针，以及 `+0x88` 处的属性对象；
   - 读取数据指针与长度（典型长度为 99 字节，对应 `x'<64 hex key><32 hex salt>'`）；
   - 使用 32 字节掩码 `d2c7442458020000004889442450488b450048844c2448488944254048584c24` 进行异或还原；
   - 提取出候选密钥后，使用 Page 1 的 HMAC-SHA512 自校验验证有效性并缓存。


## Alpha 主通行码自动提取（alpha_xkey.dll `RecoverDbKey`）

对安装包内的 `alpha_xkey.dll` 做静态逆向后确认，Alpha 的自动取 Key 链路并不依赖
`key.txt`，而是：

1. **提取 Weixin.dll 内部掩码**
   - 扫描 `Weixin.dll` 映像中的机器码模式：
     ```
     48 BA <imm64> ??? 48 BA <imm64> ??? 48 BA <imm64> ??? 48 BA <imm64> ... 48 85 C0
     ```
   - 每段 `48 BA` 是 `movabs rdx, imm64`，四段立即数按顺序拼接为 32 字节掩码；
   - 偏移关系：第 2/3/4 段分别位于前一段起始 `+0x0d..+0x12`（Alpha 会枚举
     3..8 字节的指令间隔），第四段之后 `+0x0d..+0x14` 范围内检查
     `48 85 C0`（`test rax, rax`）。
   - 对实际 205,932,072 字节的 `Weixin.dll` 验证：该算法提取到 2 个掩码，
     其中 `14619eb39b02c1b64d024aa665b7b03e459d2f2f9a5bde992d162dc798c78eec`
     位于 `RVA 0x37eb15`，其后的代码正是 `xorps xmm0/xmm1` 对 32 字节字符串
     缓冲区做原地异或，确认这就是内部 DB key 掩码。

2. **扫描 Weixin.exe 私有堆**
   - 仅枚举 `State == MEM_COMMIT` 且 `Type == MEM_PRIVATE` 的区域；
   - 按 32 字节滑动窗口匹配 MSVC x64 `std::string` 对象头：
     - `_Mysize`（+0x10）== 32
     - `_Myres`（+0x18）== 47
     - `_Bx` 未使用区（+0x06..+0x10、+0x11..+0x18、+0x19..+0x20）为零
   - 读取 `_Bx` 指针指向的 32 字节内容作为候选。

3. **异或 + 密码学验证**
   - `passphrase = candidate XOR mask`；
   - `enc_key = PBKDF2-HMAC-SHA512(passphrase, page1[0..16], 256000)`；
   - 用 `enc_key` 校验 Page 1 的 HMAC-SHA512；
   - 命中后即为 64 Hex 主通行码（32 字节原始口令），可派生全部账号数据库密钥。

Rust 实现位于 `src/db.rs` 的 `scan_master_passphrase_alpha`、`extract_internal_db_key_masks`
与 `verify_all`，并带有掩码提取、MSVC 对象头识别和 XOR→PBKDF2→HMAC 端到端单元测试。


## 图片密钥与表情解密（对齐 Alpha `GetImageKey`）

微信 4.x 的 V2 聊天图片与表情文件都依赖账号级 seed：

```text
aes_key   = md5(format!("{seed}{wxid}"))[..16]      // 16 个 ASCII 字符
xor_key   = seed & 0xff
emoji_key = md5(format!("{seed}{wxid}EMOTICON"))    // hex 解码后取前 16 字节
```

- V2 聊天图片：`.dat` 头部 15 字节 + AES-128-ECB 区 + 尾部单字节 XOR；
- 表情文件：AES-128-CBC + PKCS7，`key == IV`；
- 解密后的 `wxgf` 为 HEVC 动画容器，`jpg/png/gif` 可直接输出。

实现位于 `src/wechat/media.rs`：

- `auto_scan_image_seed_from_process`：扫描 `Weixin.exe` 私有堆中的 9~10 位十进制 seed，
  用图片模板或表情模板校验后返回 `(seed, aes_key, xor_key)`；
- `image_key_from_seed` / `emoticon_key_from_seed`：按上述公式派生；
- `decrypt_emoticon_bytes` / `decrypt_dat_aes_xor`：分别处理表情与聊天图片。
