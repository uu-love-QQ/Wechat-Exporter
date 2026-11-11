# WCDB Python 示例

这两个脚本只读取**解密后的** SQLite 文件，不负责解密微信原始库。

## 准备

```bash
pip install -r requirements.txt
```

也可以不装依赖；`read_messages.py` 会尝试调用系统的 `zstd -d`。

## 查看表结构

```bash
python inspect_wcdb.py /path/to/db_decrypted
python inspect_wcdb.py /path/to/db_decrypted --schema message/message_0.db
```

## 读取消息

```bash
python read_messages.py /path/to/db_decrypted 123456789@chatroom --limit 20
python read_messages.py /path/to/db_decrypted wxid_xxx --limit 20 --json
```

Windows PowerShell：

```powershell
python .\inspect_wcdb.py "C:\Users\你\Downloads\db_decrypted"
python .\read_messages.py "C:\Users\你\Downloads\db_decrypted" 123456789@chatroom --limit 20
```

## 注意

- 必须先 `wechat-exporter decrypt`；
- 分析时用只读方式打开；
- 真实 wxid、手机号、聊天内容不要提交到公共仓库。
