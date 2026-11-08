import { useEffect, useMemo, useState } from 'react'
import { CalendarDays, MessagesSquare, Moon, Sun, Users } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Progress } from '@/components/ui/progress'
import { Separator } from '@/components/ui/separator'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

export interface ReportKpi {
  label: string
  value: string
  note: string
}

export interface ReportSeries {
  label: string
  me: number
  other: number
}

export interface ReportCount {
  name: string
  count: number
}

export interface ReportDay {
  date: string
  count: number
  me: number
  other: number
}

export interface ReportRow {
  name: string
  id?: string
  count: number
  sent?: number
  received?: number
  media?: number
  chars?: number
  last?: string
  share?: number
}

export interface ReportWords {
  meLabel: string
  otherLabel: string
  me: Array<{ word: string; count: number }>
  other: Array<{ word: string; count: number }>
}

export interface ReportData {
  kind: 'private' | 'group' | 'dual' | 'annual'
  title: string
  heading: string
  subtitle: string
  range: string
  meta: string[]
  kpis: ReportKpi[]
  ratio: {
    me: number
    other: number
    meLabel: string
    otherLabel: string
    meAvg: number
    otherAvg: number
  } | null
  trend: ReportSeries[]
  months: ReportSeries[]
  hours: ReportSeries[]
  weekdays: ReportSeries[]
  types: ReportCount[]
  media: ReportCount[]
  missingMedia: number
  topDays: ReportDay[]
  friends: ReportRow[]
  groups: ReportRow[]
  members: ReportRow[]
  words: ReportWords | null
  generator: string
}

function pct(value: number, total: number) {
  return total > 0 ? (value / total) * 100 : 0
}

function SectionCard({
  title,
  description,
  action,
  className,
  children,
}: {
  title: string
  description?: string
  action?: React.ReactNode
  className?: string
  children: React.ReactNode
}) {
  return (
    <Card className={className}>
      <CardHeader>
        <CardTitle role="heading" aria-level={2}>
          {title}
        </CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
        {action && <CardAction>{action}</CardAction>}
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  )
}

function Legend({ labels }: { labels: [string, string] }) {
  return (
    <div className="flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
      <span className="inline-flex items-center gap-1.5">
        <span className="size-2 rounded-full bg-chart-1" />
        {labels[0]}
      </span>
      <span className="inline-flex items-center gap-1.5">
        <span className="size-2 rounded-full bg-chart-2" />
        {labels[1]}
      </span>
    </div>
  )
}

function ChartEmpty({ text = '暂无数据' }: { text?: string }) {
  return <p className="py-8 text-center text-sm text-muted-foreground">{text}</p>
}

function LineChart({ data, labels }: { data: ReportSeries[]; labels: [string, string] }) {
  const width = 820
  const height = 220
  const left = 30
  const right = 18
  const top = 18
  const bottom = 30
  const usable = width - left - right
  const max = Math.max(...data.flatMap((item) => [item.me, item.other]), 1)
  const point = (index: number, value: number) => {
    const x = data.length <= 1 ? left + usable / 2 : left + (usable * index) / (data.length - 1)
    const y = height - bottom - ((height - top - bottom) * value) / max
    return [x, y] as const
  }
  const mePoints = data.map((item, index) => point(index, item.me))
  const otherPoints = data.map((item, index) => point(index, item.other))
  const path = (points: ReadonlyArray<readonly [number, number]>) =>
    points.map(([x, y], index) => `${index === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`).join(' ')
  const mid = data[Math.floor(data.length / 2)]?.label ?? '-'
  const peak = data.reduce(
    (best, item, index) => (item.me + item.other > best.total ? { total: item.me + item.other, index } : best),
    { total: -1, index: 0 },
  )
  const peakPoint = point(peak.index, data[peak.index]?.me ?? 0)

  return (
    <div className="flex flex-col gap-3">
      <svg viewBox={`0 0 ${width} ${height}`} className="h-auto w-full" role="img" aria-label={`${labels[0]}与${labels[1]}消息趋势`}>
        {[0.25, 0.5, 0.75, 1].map((ratio) => (
          <line
            key={ratio}
            x1={left}
            x2={width - right}
            y1={height - bottom - (height - top - bottom) * ratio}
            y2={height - bottom - (height - top - bottom) * ratio}
            stroke="var(--border)"
            strokeWidth="1"
          />
        ))}
        <path d={path(mePoints)} fill="none" stroke="var(--chart-1)" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        <path
          d={path(otherPoints)}
          fill="none"
          stroke="var(--chart-2)"
          strokeWidth="2"
          strokeDasharray="5 4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        {data.length > 0 && (
          <circle cx={peakPoint[0]} cy={peakPoint[1]} r="4" fill="var(--chart-1)" stroke="var(--card)" strokeWidth="2">
            <title>{`${data[peak.index]?.label ?? ''} · ${peak.total} 条`}</title>
          </circle>
        )}
        <text x={left} y={height - 8} fill="var(--muted-foreground)" fontSize="11">
          {data[0]?.label ?? '-'}
        </text>
        <text x={width / 2} y={height - 8} fill="var(--muted-foreground)" fontSize="11" textAnchor="middle">
          {mid}
        </text>
        <text x={width - right} y={height - 8} fill="var(--muted-foreground)" fontSize="11" textAnchor="end">
          {data[data.length - 1]?.label ?? '-'}
        </text>
      </svg>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Legend labels={labels} />
        <span className="text-xs text-muted-foreground">峰值 · {peak.total} 条</span>
      </div>
    </div>
  )
}

function GroupedBars({ data, labels }: { data: ReportSeries[]; labels: [string, string] }) {
  if (data.length === 0) return <ChartEmpty />
  const max = Math.max(...data.flatMap((item) => [item.me, item.other]), 1)
  const hasOther = data.some((item) => item.other > 0)
  return (
    <div className="flex flex-col gap-3">
      <div
        className="grid h-40 items-end gap-1.5"
        style={{ gridTemplateColumns: `repeat(${data.length}, minmax(0, 1fr))` }}
        role="img"
        aria-label={`${labels[0]}与${labels[1]}时段分布`}
      >
        {data.map((item) => (
          <div
            key={item.label}
            className="flex h-full min-w-0 flex-col justify-end gap-1.5"
            title={`${item.label} · ${labels[0]} ${item.me} 条 · ${labels[1]} ${item.other} 条`}
          >
            <div className="flex h-full items-end justify-center gap-0.5">
              <span
                className="w-1/2 max-w-2 rounded-t-sm bg-chart-1"
                style={{ height: `${Math.max(2, pct(item.me, max))}%` }}
              />
              {hasOther && (
                <span
                  className="w-1/2 max-w-2 rounded-t-sm bg-chart-2/80"
                  style={{ height: `${Math.max(2, pct(item.other, max))}%` }}
                />
              )}
            </div>
            <span className="truncate text-center text-[8px] text-muted-foreground">{item.label}</span>
          </div>
        ))}
      </div>
      <Legend labels={labels} />
    </div>
  )
}

function TypeList({ types, total }: { types: ReportCount[]; total: number }) {
  if (types.length === 0) return <ChartEmpty />
  const max = Math.max(...types.map((item) => item.count), 1)
  return (
    <div className="flex flex-col gap-3">
      {types.map((item) => (
        <div key={item.name} className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-3 text-xs">
            <span className="truncate">{item.name}</span>
            <span className="shrink-0 text-muted-foreground tabular-nums">
              {item.count} · {Math.round(pct(item.count, total))}%
            </span>
          </div>
          <Progress value={pct(item.count, max)} />
        </div>
      ))}
    </div>
  )
}

function TopDaysTable({ days }: { days: ReportDay[] }) {
  if (days.length === 0) return <ChartEmpty />
  const max = Math.max(...days.map((item) => item.count), 1)
  return (
    <div className="overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-10">#</TableHead>
            <TableHead>日期</TableHead>
            <TableHead>消息</TableHead>
            <TableHead className="w-[38%]">占比</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {days.map((item, index) => (
            <TableRow key={`${item.date}-${index}`}>
              <TableCell className="text-muted-foreground tabular-nums">{index + 1}</TableCell>
              <TableCell className="whitespace-nowrap tabular-nums">{item.date}</TableCell>
              <TableCell className="whitespace-nowrap tabular-nums">
                {item.count}
                <span className="ml-2 text-xs text-muted-foreground">
                  {item.me} / {item.other}
                </span>
              </TableCell>
              <TableCell>
                <Progress value={pct(item.count, max)} />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

function RankTable({ rows, kind }: { rows: ReportRow[]; kind: 'friends' | 'members' | 'groups' }) {
  if (rows.length === 0) return null
  return (
    <div className="overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{kind === 'groups' ? '群聊' : kind === 'members' ? '成员' : '好友'}</TableHead>
            {kind !== 'groups' && <TableHead className="hidden sm:table-cell">账号</TableHead>}
            <TableHead className="text-right">消息</TableHead>
            {kind === 'friends' && (
              <>
                <TableHead className="hidden text-right md:table-cell">我</TableHead>
                <TableHead className="hidden text-right md:table-cell">对方</TableHead>
              </>
            )}
            {kind === 'members' && (
              <>
                <TableHead className="hidden text-right md:table-cell">媒体</TableHead>
                <TableHead className="hidden text-right lg:table-cell">字数</TableHead>
                <TableHead className="text-right">占比</TableHead>
              </>
            )}
            <TableHead className="hidden text-right sm:table-cell">最后活跃</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row) => (
            <TableRow key={`${row.name}-${row.id ?? ''}`}>
              <TableCell className="max-w-40 truncate font-medium">{row.name}</TableCell>
              {kind !== 'groups' && (
                <TableCell className="hidden max-w-48 truncate font-mono text-xs text-muted-foreground sm:table-cell">
                  {row.id}
                </TableCell>
              )}
              <TableCell className="text-right tabular-nums">{row.count}</TableCell>
              {kind === 'friends' && (
                <>
                  <TableCell className="hidden text-right tabular-nums md:table-cell">{row.sent ?? 0}</TableCell>
                  <TableCell className="hidden text-right tabular-nums md:table-cell">{row.received ?? 0}</TableCell>
                </>
              )}
              {kind === 'members' && (
                <>
                  <TableCell className="hidden text-right tabular-nums md:table-cell">{row.media ?? 0}</TableCell>
                  <TableCell className="text-right tabular-nums">
                    {row.share === undefined ? '-' : `${Math.round(row.share * 100)}%`}
                  </TableCell>
                </>
              )}
              <TableCell className="hidden whitespace-nowrap text-right text-xs text-muted-foreground sm:table-cell">
                {row.last ?? '-'}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

function WordCloud({ words }: { words: Array<{ word: string; count: number }> }) {
  if (words.length === 0) return <ChartEmpty text="暂无高频词" />
  const max = Math.max(...words.map((item) => item.count), 1)
  return (
    <div className="flex flex-wrap items-center gap-2">
      {words.map((item, index) => (
        <Badge
          key={item.word}
          variant={index < 4 ? 'default' : 'secondary'}
          className="rounded-lg py-1"
          style={{ fontSize: `${12 + (item.count / max) * 8}px` }}
          title={`${item.word} · ${item.count} 次`}
        >
          {item.word}
          <span className="opacity-60">{item.count}</span>
        </Badge>
      ))}
    </div>
  )
}

function KpiCard({ kpi }: { kpi: ReportKpi }) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardDescription>{kpi.label}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-0.5">
        <span className="text-2xl font-semibold tracking-tight tabular-nums">{kpi.value}</span>
        <span className="truncate text-xs text-muted-foreground">{kpi.note}</span>
      </CardContent>
    </Card>
  )
}

export function ReportApp({ data }: { data: ReportData }) {
  const [dark, setDark] = useState(() => document.documentElement.classList.contains('dark'))
  const total = useMemo(() => data.ratio ? data.ratio.me + data.ratio.other : 0, [data.ratio])

  useEffect(() => {
    document.documentElement.classList.toggle('dark', dark)
    try {
      localStorage.setItem('wechat-report-theme', dark ? 'dark' : 'light')
    } catch {
      // ignore storage failures
    }
  }, [dark])

  const ratioMe = data.ratio ? pct(data.ratio.me, total) : 0
  const ratioOther = data.ratio ? pct(data.ratio.other, total) : 0
  const hasTrendTabs = data.trend.length > 0 && data.months.length > 0
  const labels: [string, string] = [data.ratio?.meLabel ?? '我', data.ratio?.otherLabel ?? '其他人']

  return (
    <main className="mx-auto flex w-full max-w-6xl flex-col gap-5 p-4 md:p-8">
      <div className="flex justify-end">
        <Button
          variant="outline"
          size="icon"
          aria-label={dark ? '切换到浅色主题' : '切换到深色主题'}
          onClick={() => setDark((value) => !value)}
        >
          {dark ? <Sun data-icon="inline-start" aria-hidden="true" /> : <Moon data-icon="inline-start" aria-hidden="true" />}
        </Button>
      </div>

      <header className="relative overflow-hidden rounded-3xl bg-primary p-6 text-primary-foreground shadow-sm md:p-9">
        <div className="pointer-events-none absolute -right-16 -top-20 size-64 rounded-full border border-primary-foreground/20 bg-primary-foreground/10" />
        <div className="pointer-events-none absolute -bottom-24 right-24 size-56 rounded-full bg-primary-foreground/5" />
        <div className="relative flex flex-col gap-2">
          <div className="flex items-center gap-2 text-xs font-medium tracking-[0.16em] uppercase opacity-75">
            <MessagesSquare data-icon="inline-start" aria-hidden="true" />
            {data.kind === 'group' ? 'Group chat analytics' : data.kind === 'annual' ? 'Annual chat analytics' : 'Chat analytics'}
          </div>
          <h1 className="max-w-3xl text-2xl font-semibold tracking-tight md:text-3xl">{data.heading}</h1>
          <p className="max-w-3xl text-sm opacity-85">{data.subtitle}</p>
          <div className="mt-2 inline-flex w-fit rounded-full bg-primary-foreground/15 px-3 py-1 text-xs">
            {data.range}
          </div>
          <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs opacity-80">
            {data.meta.map((item) => (
              <span key={item}>{item}</span>
            ))}
          </div>
        </div>
      </header>

      <section aria-label="核心指标" className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        {data.kpis.map((kpi) => (
          <KpiCard key={kpi.label} kpi={kpi} />
        ))}
      </section>

      {data.ratio && (
        <SectionCard
          title="我们的对话比例"
          description={`共 ${total} 条消息`}
          action={
            <Badge variant="secondary">
              {data.ratio.meLabel} {Math.round(ratioMe)}%
            </Badge>
          }
        >
          <div className="flex flex-col gap-3">
            <div className="flex h-10 overflow-hidden rounded-full bg-muted" role="img" aria-label={`${data.ratio.meLabel} ${Math.round(ratioMe)}%，${data.ratio.otherLabel} ${Math.round(ratioOther)}%`}>
              <div
                className="flex min-w-0 items-center justify-center bg-chart-1 px-2 text-xs font-medium whitespace-nowrap text-primary-foreground"
                style={{ width: `${ratioMe}%` }}
              >
                {ratioMe > 12 ? `${data.ratio.meLabel} ${Math.round(ratioMe)}%` : ''}
              </div>
              <div
                className="flex min-w-0 flex-1 items-center justify-center bg-chart-2 px-2 text-xs font-medium whitespace-nowrap text-primary-foreground"
                style={{ width: `${ratioOther}%` }}
              >
                {ratioOther > 12 ? `${data.ratio.otherLabel} ${Math.round(ratioOther)}%` : ''}
              </div>
            </div>
            <div className="flex flex-wrap justify-between gap-2 text-xs text-muted-foreground">
              <span>{data.ratio.meLabel} · {data.ratio.me} 条 · 平均每条 {data.ratio.meAvg.toFixed(1)} 字</span>
              <span>{data.ratio.otherLabel} · {data.ratio.other} 条 · 平均每条 {data.ratio.otherAvg.toFixed(1)} 字</span>
            </div>
          </div>
        </SectionCard>
      )}

      {(data.trend.length > 0 || data.months.length > 0) && (
        <SectionCard
          title="消息趋势"
          description="时间维度上的消息量变化"
          action={
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <CalendarDays aria-hidden="true" />
              {hasTrendTabs ? '每日 / 月度' : data.months.length > 0 ? '月度' : '每日'}
            </div>
          }
        >
          {hasTrendTabs ? (
            <Tabs defaultValue="daily" className="gap-4">
              <TabsList>
                <TabsTrigger value="daily">每日</TabsTrigger>
                <TabsTrigger value="monthly">月度</TabsTrigger>
              </TabsList>
              <TabsContent value="daily">
                <LineChart data={data.trend} labels={labels} />
              </TabsContent>
              <TabsContent value="monthly">
                <LineChart data={data.months} labels={labels} />
              </TabsContent>
            </Tabs>
          ) : data.months.length > 0 ? (
            <LineChart data={data.months} labels={labels} />
          ) : (
            <LineChart data={data.trend} labels={labels} />
          )}
        </SectionCard>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        {data.hours.length > 0 && (
          <SectionCard title="一天中的分布" description="按 24 小时统计消息量">
            <GroupedBars data={data.hours} labels={labels} />
          </SectionCard>
        )}
        {data.weekdays.length > 0 && (
          <SectionCard title="一周中的分布" description="按星期统计消息量">
            <GroupedBars data={data.weekdays} labels={labels} />
          </SectionCard>
        )}
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        {data.types.length > 0 && (
          <SectionCard title="消息类型" description="各类型消息占比">
            <TypeList types={data.types} total={data.types.reduce((sum, item) => sum + item.count, 0)} />
          </SectionCard>
        )}
        {data.topDays.length > 0 && (
          <SectionCard title="最活跃的日子" description="按单日消息数排序">
            <TopDaysTable days={data.topDays} />
          </SectionCard>
        )}
      </div>

      {data.friends.length > 0 && (
        <SectionCard title={data.kind === 'annual' ? '好友消息排行' : '好友消息排行'} description="消息数与双方发送量">
          <RankTable rows={data.friends} kind="friends" />
        </SectionCard>
      )}

      {data.groups.length > 0 && (
        <SectionCard title="群聊消息排行" description="群聊消息量与最后活跃时间">
          <RankTable rows={data.groups} kind="groups" />
        </SectionCard>
      )}

      {data.members.length > 0 && (
        <SectionCard title="群成员详细信息" description="成员发言、媒体与最后活跃时间">
          <RankTable rows={data.members} kind="members" />
        </SectionCard>
      )}

      {data.media.length > 0 && (
        <SectionCard title="媒体内容" description={data.missingMedia > 0 ? `其中 ${data.missingMedia} 个未在本地索引到` : '已索引媒体文件'}>
          <div className="flex flex-wrap gap-2">
            {data.media.map((item) => (
              <Badge key={item.name} variant="outline" className="px-3 py-1.5">
                {item.name}
                <span className="text-muted-foreground">{item.count}</span>
              </Badge>
            ))}
          </div>
        </SectionCard>
      )}

      {data.words && (
        <SectionCard
          title="我们的高频词"
          description="基于文本消息样本"
          action={
            <Badge variant="secondary">
              <Users data-icon="inline-start" aria-hidden="true" />
              {data.words.meLabel} / {data.words.otherLabel}
            </Badge>
          }
        >
          <Tabs defaultValue="me" className="gap-4">
            <TabsList>
              <TabsTrigger value="me">{data.words.meLabel}最常说的</TabsTrigger>
              <TabsTrigger value="other">{data.words.otherLabel}最常说的</TabsTrigger>
            </TabsList>
            <TabsContent value="me">
              <WordCloud words={data.words.me} />
            </TabsContent>
            <TabsContent value="other">
              <WordCloud words={data.words.other} />
            </TabsContent>
          </Tabs>
        </SectionCard>
      )}

      <Separator />
      <footer className="flex flex-col gap-1 pb-4 text-center text-xs text-muted-foreground">
        <span>由 {data.generator} 生成 · 静态离线报告</span>
      </footer>
    </main>
  )
}
