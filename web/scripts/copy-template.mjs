import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

const pairs = [
  ['dist/index.html', '../src/export/chat_template.html'],
  ['dist-report/report.html', '../src/export/report_template.html'],
]
for (const [from, to] of pairs) {
  const source = resolve(from)
  const target = resolve(to)
  mkdirSync(dirname(target), { recursive: true })
  copyFileSync(source, target)
  console.log(`copied ${source} -> ${target}`)
}
