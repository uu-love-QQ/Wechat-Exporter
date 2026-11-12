# Changelog

## 1.0.1

- 年度报告支持 `--png`，自动调用本机 Chrome/Edge/Chromium 生成完整长图；
- 新增 `--browser` 和 `WECHAT_EXPORTER_BROWSER` 指定浏览器；
- PNG 自动裁剪底部空白；
- 仅年度报告允许 `--png`，其他报告类型会给出明确错误。

## 1.0.0

首个稳定版本。

### CLI

- `conversation`：列出会话，`--export` 导出会话；
- `contact`：好友、群、公众号、服务号、企业微信、前好友、黑名单；
- `search`：跨会话全文检索，支持 zstd；
- `report`：私聊、群聊、双人、年度报告；
- `moments`：朋友圈统计、时间线、点赞/评论；
- `favorite`：收藏列表与导出；
- `decrypt`：SQLCipher 数据库转为普通 SQLite，带交互进度和 `--silent`；
- `recovery`：完整库写回、单会话 `backup.zip` 写回、离线恢复；
- `backup`：WebDAV 配置、测试、查看、清除；
- `upgrade`：自更新。

### 导出

- JSON、Markdown、ChatLab JSON/JSONL、HTML、TXT、XLSX、Arkme JSON、SQL、SQLite、WeClone CSV；
- HTML 离线搜索、引用跳转、图片灯箱、聊天记录详情；
- 图片、语音、视频、表情、文件媒体导出；
- 引用、公众号文章、视频号、拍一拍、系统消息类型修复；
- 普通 `--format sqlite` 与会话级 `conversation.sqlite`。

### 备份与恢复

- `conversation --export --backup`：生成 `backup.zip`，包含 `backup.db`、HTML、媒体；
- `backup.db` 保存 WCDB 原始字段；
- `recovery` 支持按会话、按时间范围部分恢复；
- 完整 `decrypt` 备份支持整库恢复；
- 恢复前自动备份目标数据库。

### WebDAV

- `--upload` 将输出目录打包为 ZIP 后上传；
- 默认 100 MB 分卷，`--upload-part-size` 可调整，`0` 表示不分卷；
- 分卷并发上传，最后上传 `*.parts.json` 清单。

### 文档

- `docs/WCDB.md`：零基础教程、SQLCipher、WAL、zstd、XML、消息类型、Python 实战；
- `docs/databases/`：按数据库拆分的详细 DDL、字段说明和示例；
- `docs/examples/`：可直接运行的 Python 脚本；
- 真实 `db_storage` 采样得到的字段差异和特殊处理说明。

### 修复

- 引用、公众号文章、视频号被 `<patMsg>` 误判为拍一拍/系统消息；
- 报告过滤误判；
- WebDAV 逐文件上传慢；
- `decrypt` 无进度条。
