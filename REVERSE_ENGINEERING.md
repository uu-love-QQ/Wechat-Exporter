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
export/mod.rs       ten format writers + report
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
