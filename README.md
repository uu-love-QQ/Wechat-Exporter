# Wechat-Exporter

微信 4.x 本地数据库解密、全文检索与多格式聊天记录导出独立工具（纯 Rust 跨平台实现）。
基于标准 SQLCipher 4 密码学算法与内存扫描技术直接解密微信本地存储，独立运行于 **Windows** 与 **Linux** 两端。
---

## 一、双端构建产物

项目中已配置好交叉编译工具链并生成了独立 Release 二进制：

- **Windows 64 位原生二进制**：
  `wechat-exporter-rs/target/x86_64-pc-windows-gnu/release/wechat-exporter.exe`
- **Linux 64 位原生二进制**：
  `wechat-exporter-rs/target/release/wechat-exporter`

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
wechat-exporter --detect-databases
# 简写
wechat-exporter -D

# 自动选用最新活跃账号列出会话列表（无需输入路径）
wechat-exporter --key <64位Hex密钥> --list-conversations
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
.\wechat-exporter.exe -D

# 2. 列出最近活跃会话（自动关联电脑管家迁移路径，传入主口令）
.\wechat-exporter.exe --key <64位Hex主通行码> --list-conversations

# 3. 全文检索关键词
.\wechat-exporter.exe --key <64位Hex主通行码> --search "合同" --ascending

# 4. 导出指定会话（含图片自动解密为真实图片格式）：
.\wechat-exporter.exe `
  --key <64位Hex主通行码> `
  --conversation-id "wxid_85da986onp4h22" `
  --format all `
  --include-media `
  --output-dir "C:\Users\14744\Downloads\uuMovies\test"

# 5. 全自动模式（微信登录在线时，无需提供路径与 key）：
.\wechat-exporter.exe `
  --conversation-id "wxid_85da986onp4h22" `
  --format all `
  --include-media `
  --output-dir "C:\Users\14744\Downloads\uuMovies\test"
```

## 私有文件 CDN 鉴权（novac2c）

微信 4.x 的私有文件（PDF/ZIP/APK 等）只给出：

```xml
<appattach>
  <cdnattachurl>3057...</cdnattachurl>
  <aeskey>...</aeskey>
</appattach>
```

真实下载地址是 `https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_param=<cdnattachurl>`，
但腾讯强制要求同请求携带 `authkey`（Hex 编码的 ASN.1 DER 票据）。实测服务端行为：

| 请求 | 响应 |
|---|---|
| 不带 `authkey` | `403 authorization error!` |
| `authkey` 为空 | `400 authkey param is empty` |
| `authkey` 非 ASN.1 | `400 plain authkey ASN decode failed` |

因此本工具提供多层凭据来源，按优先级使用：

1. 从运行中的 `Weixin.exe` 内存扫描到的 `authkey` 候选（ASN.1 DER 有效者优先）；
2. 消息 XML 自带的 `authkey` / `<tpauthkey>`（部分消息会携带）；
3. 命令行 `--authkey <HEX>`；
4. `.wechat-exporter.json` 中由 `init` 捕获的候选列表。

登录凭据里的 `authkey`（常见形态是以 `@im.wxwork` 结尾的 protobuf，如
`0A2B...10...`）**不是** CDN 票据：服务端会以 `plain authkey ASN decode failed` 拒绝。
真正的票据是 `GetCDNDns` 路由信息中的 `authkey`（`authkey/frontip/zoneip` 三元组），
以 Hex 编码的 ASN.1 DER 存在。扫描时用完整 DER 解析校验（顶层 `SEQUENCE`、长度闭合），
并把所有候选按「DER 有效优先」排序；下载时逐个尝试，成功的票据在本次运行内缓存。

`--dump-session` 会只扫描并打印当前微信进程的 Cookie 字段与全部 `authkey` 候选
（含 `asn1=是/否` 标记），不会读取数据库。扫描按「小内存区优先」进行，默认 30 秒上限，
每 2 秒打印已扫描 MB 数，避免微信多 GB 堆导致卡死。拿到 `asn1=是` 的票据后可用
`--authkey <HEX>` 手动注入验证。

导出默认**不扫描**进程内存（直接使用配置中的候选），避免每次导出卡顿；
需要刷新时加 `--scan-session`（20 秒上限）或先运行 `init` / `--dump-session`。

同时 `--cookie "wxuin=..; wxsid=..; skey=..; pass_ticket=.."` 可手动注入登录 Cookie；
`init` 会从运行中的 `Weixin.exe` 私有堆扫描并保存 Cookie 与全部 authkey 候选。

> 说明：`authkey` 由微信客户端在与腾讯服务器完成 MMTLS/CDN DNS 交互后签发。对于从未在
> 客户端触发过下载的文件，内存中可能不存在对应票据；此时导出会记录服务端错误码
> （`x-error-code` / `x-error-message`）而不是写入损坏文件。


### 媒体源优先级（对齐 Alpha 实测）

| 消息类型 | 原图字段 | 说明 |
|---|---|---|
| 表情 `<emoji>` | `cdnurl` | **明文原图**，字节数与 XML `len` 相等，MD5 等于消息 `md5` |
|  | `encrypturl` | AES 加密变体 |
|  | `externurl` | AES 加密的 wxgf 预览（体积最小） |
| 图片 `<img>` | `cdnbigimgurl` | 原图 |
|  | `cdnmidimgurl` | 中图 |
|  | `cdnthumburl` | 缩略图 |
| 私有文件 `<appattach>` | `cdnattachurl` | 需 `authkey` 签名的 ASN.1 定位符 |

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

