import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import { ReportApp, type ReportData } from './ReportApp'

const node = document.getElementById('report-data')
let data: ReportData | null = null
try {
  data = node?.textContent ? (JSON.parse(node.textContent) as ReportData) : null
} catch (error) {
  console.error('invalid report data', error)
}

if (data) {
  document.title = data.title
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {data ? (
      <ReportApp data={data} />
    ) : (
      <main className="mx-auto max-w-3xl p-8 text-sm text-muted-foreground">
        报告数据为空或无法解析。
      </main>
    )}
  </StrictMode>,
)
