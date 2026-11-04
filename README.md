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

因此本工具提供三层凭据来源，按优先级使用：

1. 消息 XML 自带的 `authkey` / `<tpauthkey>`（部分消息会携带）；
2. 命令行 `--authkey <HEX>`；
3. `.wechat-exporter.json` 中由 `init` 捕获的会话凭据。

同时 `--cookie "wxuin=..; wxsid=..; skey=..; pass_ticket=.."` 可手动注入登录 Cookie；
`init` 会从运行中的 `Weixin.exe` 私有堆扫描并保存 Cookie 与 authkey，之后运行完全离线可用。

> 说明：`authkey` 由微信客户端在与腾讯服务器完成 MMTLS 交互后签发。对于从未在客户端
> 触发过下载的文件，内存中可能不存在对应票据；此时导出会记录失败原因而不是写入损坏文件。

