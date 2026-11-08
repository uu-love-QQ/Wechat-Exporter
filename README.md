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

### HTML 界面与引用跳转

HTML 导出由 `web/` 前端模板生成（Vite + React 19 + Tailwind v4 + shadcn/ui），
构建为单文件 HTML 后由 Rust 注入会话 JSON。修改 UI 后重新生成模板：

```bash
cd web
npm install
npm run export:template   # 构建并复制到 src/export/chat_template.html
```

HTML 导出是自包含的响应式聊天界面：吸顶标题栏、即时搜索（`/` 聚焦、`Esc` 清空）、
亮/暗主题切换、按日期分组、左右消息气泡、图片/音频/视频内嵌、缺失附件提示与“回到最新”按钮。

每条消息写入稳定锚点 `id="msg-N"`。引用消息（type 57）解析
`<refermsg><svrid>`，在导出集合内找到原消息时渲染可点击的 `#msg-N` 链接；
找不到（已删除、超出时间范围或分片缺失）时显示
引用块始终优先展示原发送者与原内容；原消息不在导出集合时，仅以弱提示标注
「原消息不在导出范围」，不生成悬空链接。
引用块同时显示原发送者与原消息摘要，回复正文独立渲染。

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

