# Wechat-Exporter HTML template

Vite + React 19 + Tailwind v4 + shadcn/ui source for the self-contained HTML export.

```bash
npm install
npm run export:template   # builds dist/index.html and copies it to ../src/export/chat_template.html
```

Rust replaces the `__CHAT_DATA__` JSON island with the exported conversation.
The template is committed so `cargo build` does not require Node.
