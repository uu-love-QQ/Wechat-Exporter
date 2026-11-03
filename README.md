# Wechat-Exporter

微信 4.x 本地数据库解密、全文检索与多格式聊天记录导出工具（Rust 跨平台 1:1 复刻版）。

基于对 `Alpha-0.3.5-x64-Setup.exe` 内部二进制（`wechat-exporter.exe`、`alpha_native.dll`、`alpha_xkey.dll`、`alpha-engine.exe`）的反汇编逆向与架构分析重写。无需依赖 Windows 平台特定的 Alpha 守护进程或私有安全运行时（DRM），即可在 **Windows** 与 **Linux** 两端独立运行。

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

### 1. 数据库密钥自动获取优先级
1. **优先从运行中的微信内存自动提取**（Windows 环境）：
   当未传入 `--key` 时，CLI 自动调用 Windows API 打开 `Weixin.exe` 进程，在内存中扫描 64 字节 Hex 口令与 `x'<hex>'` 缓存模式，并使用第一页的 HMAC 签名进行即时验证。验证通过后自动解密，无需手动输入。
2. **自动读取当前目录 `key.txt`**：
   若当前运行目录下存在 `key.txt` 且包含 64 位 Hex 密钥，自动加载生效。
3. **命令行传入保底**：
   通过 `--key <64-HEX>` 或 stdin 管道输入。

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
.\wechat-exporter.exe --key 22f2af8d4301484db4defe2fe89a972417944e0538a44fb3a7213db93149c36e --list-conversations

# 3. 全文检索关键词
.\wechat-exporter.exe --key 22f2af8d4301484db4defe2fe89a972417944e0538a44fb3a7213db93149c36e --search "合同" --ascending

# 4. 导出指定会话（含图片自动解密为真实图片格式）：
.\wechat-exporter.exe `
  --key 22f2af8d4301484db4defe2fe89a972417944e0538a44fb3a7213db93149c36e `
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
