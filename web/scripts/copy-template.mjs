import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

const source = resolve('dist/index.html')
const target = resolve('../src/export/chat_template.html')
mkdirSync(dirname(target), { recursive: true })
copyFileSync(source, target)
console.log(`copied ${source} -> ${target}`)
