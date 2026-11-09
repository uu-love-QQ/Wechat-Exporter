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
import { cn } from '@/lib/utils'

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

export interface AnnualPerson {
  conversationId: string
  displayName: string
  messages: number
  sent?: number
  received?: number
}

export interface AnnualCompanion {
  period: string
  conversationId: string
  displayName: string
  messages: number
  periodMessages: number
}

export interface AnnualData {
  topFriends: AnnualPerson[]
  monthlyCompanions: AnnualCompanion[]
  heatmap: {
    weekdayHourCounts: number[][]
    peak: { weekday: number; hour: number; messages: number } | null
  }
  nightCompanion: { conversationId: string; displayName: string; nightMessages: number; share: number } | null
  nightShareOfAll: number | null
  initiative: {
    startsMine: number
    startsPeer: number
    rateMine: number | null
    topInitiated: { conversationId: string; displayName: string; startsMine: number; startsPeer: number } | null
  }
  response: {
    mine: { averageMs: number; count: number } | null
    peer: { averageMs: number; count: number } | null
    fastestFriend: { conversationId: string; displayName: string; averageMs: number; replies: number } | null
  }
  balancedFriend: { conversationId: string; displayName: string; sent: number; received: number; balance: number } | null
  streak: { conversationId: string; displayName: string; days: number; startDate: string; endDate: string } | null
  accountStreak: { days: number; startDate: string; endDate: string } | null
  busiestDay: { date: string; messages: number; topFriend: { displayName: string; messages: number } | null } | null
  fading: { conversationId: string; displayName: string; earlyMessages: number; lateMessages: number } | null
  phrases: Array<{ text: string; count: number }>
  summary: Array<{ label: string; value: string; person?: string }>
  moments: null
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
  annual?: AnnualData
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
    <div className="flex flex-wrap items-start gap-2">
      {words.map((item, index) => {
        const size = 12 + (item.count / max) * 6
        return (
          <Badge
            key={item.word}
            variant={index < 3 ? 'default' : index < 10 ? 'secondary' : 'outline'}
            className={cn(
              'h-auto max-w-full rounded-lg py-1 whitespace-nowrap shadow-none',
              index < 3 && 'border-transparent',
            )}
            style={{ fontSize: `${size}px` }}
            title={`${item.word} · ${item.count} 次`}
          >
            <span className="truncate">{item.word}</span>
            <span className="text-[0.7em] opacity-60 tabular-nums">{item.count}</span>
          </Badge>
        )
      })}
    </div>
  )
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} 毫秒`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} 秒`
  if (ms < 3_600_000) return `${(ms / 60_000).toFixed(1)} 分钟`
  return `${(ms / 3_600_000).toFixed(1)} 小时`
}

function AnnualSections({ annual }: { annual: AnnualData }) {
  const maxHeat = Math.max(...annual.heatmap.weekdayHourCounts.flat(), 1)
  const weekdays = ['一', '二', '三', '四', '五', '六', '日']
  return (
    <>
      <SectionCard title="聊得最多的人" description="只统计一对一的私聊消息">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>好友</TableHead>
                <TableHead className="text-right">消息</TableHead>
                <TableHead className="hidden text-right sm:table-cell">我发</TableHead>
                <TableHead className="hidden text-right sm:table-cell">TA 发</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {annual.topFriends.slice(0, 15).map((friend) => (
                <TableRow key={friend.conversationId}>
                  <TableCell className="font-medium">{friend.displayName}</TableCell>
                  <TableCell className="text-right tabular-nums">{friend.messages}</TableCell>
                  <TableCell className="hidden text-right tabular-nums sm:table-cell">{friend.sent ?? 0}</TableCell>
                  <TableCell className="hidden text-right tabular-nums sm:table-cell">{friend.received ?? 0}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </SectionCard>

      <div className="grid gap-4 lg:grid-cols-2">
        <SectionCard title="每个月陪你最多的人" description="按月统计私聊消息最多的好友">
          <div className="flex flex-col gap-2">
            {annual.monthlyCompanions.slice(-24).map((item) => (
              <div key={item.period} className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 text-sm">
                <span className="shrink-0 text-muted-foreground tabular-nums">{item.period}</span>
                <span className="min-w-0 flex-1 truncate">{item.displayName}</span>
                <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                  {item.messages} / {item.periodMessages}
                </span>
              </div>
            ))}
          </div>
        </SectionCard>

        <SectionCard title="什么时候最常聊天" description="星期 × 小时的私聊消息热力图">
          <div className="flex flex-col gap-2">
            <div className="grid gap-1" style={{ gridTemplateColumns: 'auto repeat(24, minmax(0, 1fr))' }}>
              <span />
              {Array.from({ length: 24 }, (_, hour) => (
                <span key={hour} className="text-center text-[8px] text-muted-foreground">
                  {hour % 6 === 0 ? hour : ''}
                </span>
              ))}
              {annual.heatmap.weekdayHourCounts.map((row, weekday) => (
                <div key={weekday} className="contents">
                  <span className="pr-1 text-[10px] text-muted-foreground">{weekdays[weekday]}</span>
                  {row.map((count, hour) => (
                    <span
                      key={hour}
                      className="aspect-square rounded-[2px] bg-chart-1"
                      style={{ opacity: Math.max(0.06, count / maxHeat) }}
                      title={`周${weekdays[weekday]} ${hour}:00 · ${count} 条`}
                    />
                  ))}
                </div>
              ))}
            </div>
            <p className="text-xs text-muted-foreground">
              {annual.heatmap.peak
                ? `最常聊天：周${weekdays[annual.heatmap.peak.weekday]} ${annual.heatmap.peak.hour}:00 · ${annual.heatmap.peak.messages} 条`
                : '暂无时段数据'}
              {annual.nightCompanion
                ? ` · 深夜最多：${annual.nightCompanion.displayName} ${annual.nightCompanion.nightMessages} 条`
                : ''}
            </p>
          </div>
        </SectionCard>
      </div>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        <SectionCard title="谁先开口" description="间隔超过 1 小时算一段新对话">
          <div className="flex flex-col gap-2 text-sm">
            <span className="text-2xl font-semibold tabular-nums">
              {annual.initiative.rateMine === null ? '-' : `${Math.round(annual.initiative.rateMine * 100)}%`}
            </span>
            <span className="text-xs text-muted-foreground">
              你 {annual.initiative.startsMine} 段 · 好友 {annual.initiative.startsPeer} 段
            </span>
            {annual.initiative.topInitiated && (
              <span className="text-xs text-muted-foreground">
                最常主动找：{annual.initiative.topInitiated.displayName} · {annual.initiative.topInitiated.startsMine} 次
              </span>
            )}
          </div>
        </SectionCard>
        <SectionCard title="回得多快" description="对方消息后的下一条，间隔不超过 24 小时">
          <div className="flex flex-col gap-1 text-sm">
            <span>你回好友：{annual.response.mine ? formatDuration(annual.response.mine.averageMs) : '-'}</span>
            <span>好友回你：{annual.response.peer ? formatDuration(annual.response.peer.averageMs) : '-'}</span>
            {annual.response.fastestFriend && (
              <span className="text-xs text-muted-foreground">
                最快：{annual.response.fastestFriend.displayName} · {annual.response.fastestFriend.replies} 次
              </span>
            )}
          </div>
        </SectionCard>
        <SectionCard title="最有来有往的人" description="双方都发过 50 条以上的好友中收发最接近">
          {annual.balancedFriend ? (
            <div className="flex flex-col gap-1 text-sm">
              <span className="text-lg font-medium">{annual.balancedFriend.displayName}</span>
              <span className="text-xs text-muted-foreground">
                你发 {annual.balancedFriend.sent} · TA 发 {annual.balancedFriend.received} · 平衡度{' '}
                {Math.round(annual.balancedFriend.balance * 100)}%
              </span>
            </div>
          ) : (
            <ChartEmpty text="暂无双方都发过 50 条以上的好友" />
          )}
        </SectionCard>
        <SectionCard title="连续与高峰" description="每天至少一条消息才算连续">
          <div className="flex flex-col gap-1 text-sm">
            <span>
              {annual.streak
                ? `和 ${annual.streak.displayName} 连续 ${annual.streak.days} 天`
                : '暂无连续 2 天以上的好友'}
            </span>
            {annual.accountStreak && <span className="text-xs text-muted-foreground">自己最长连续 {annual.accountStreak.days} 天</span>}
            {annual.busiestDay && (
              <span className="text-xs text-muted-foreground">
                最多一天：{annual.busiestDay.date} · {annual.busiestDay.messages} 条
                {annual.busiestDay.topFriend ? ` · ${annual.busiestDay.topFriend.displayName}` : ''}
              </span>
            )}
          </div>
        </SectionCard>
        <SectionCard title="渐渐少聊的人" description="前段至少 80/100 条且超过后段 5 倍">
          {annual.fading ? (
            <div className="flex flex-col gap-1 text-sm">
              <span className="text-lg font-medium">{annual.fading.displayName}</span>
              <span className="text-xs text-muted-foreground">
                前段 {annual.fading.earlyMessages} 条 · 后段 {annual.fading.lateMessages} 条
              </span>
            </div>
          ) : (
            <ChartEmpty text="没有发现明显的降温好友" />
          )}
        </SectionCard>
        <SectionCard title="你的口头禅" description="自己发出的 2–20 字文本，至少出现两次">
          {annual.phrases.length > 0 ? (
            <div className="flex flex-col gap-2">
              {annual.phrases.slice(0, 8).map((item, index) => (
                <div key={item.text} className="flex items-center justify-between gap-2 text-sm">
                  <span className="min-w-0 truncate">
                    {index + 1}. {item.text}
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">{item.count} 次</span>
                </div>
              ))}
            </div>
          ) : (
            <ChartEmpty text="没有重复出现的短句" />
          )}
        </SectionCard>
      </div>

      <SectionCard title="朋友圈" description="按发布时间与点赞统计">
        <ChartEmpty text="当前数据源没有读取到朋友圈数据" />
      </SectionCard>

      {annual.summary.length > 0 && (
        <SectionCard title="尾声" description="一页看完">
          <div className="grid gap-2 sm:grid-cols-2">
            {annual.summary.map((item) => (
              <div key={`${item.label}-${item.value}`} className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 text-sm">
                <span className="text-muted-foreground">{item.label}</span>
                <span className="truncate text-right">
                  {item.value}
                  {item.person ? ` · ${item.person}` : ''}
                </span>
              </div>
            ))}
          </div>
        </SectionCard>
      )}
    </>
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

      <header className="relative overflow-hidden rounded-3xl bg-[var(--report-hero)] p-6 text-[var(--report-hero-fg)] shadow-sm md:p-9">
        <div className="pointer-events-none absolute -right-16 -top-20 size-64 rounded-full border border-[var(--report-hero-fg)]/20 bg-[var(--report-hero-fg)]/10" />
        <div className="pointer-events-none absolute -bottom-24 right-24 size-56 rounded-full bg-[var(--report-hero-fg)]/5" />
        <div className="relative flex flex-col gap-2">
          <div className="flex items-center gap-2 text-xs font-medium tracking-[0.16em] uppercase opacity-75">
            <MessagesSquare data-icon="inline-start" aria-hidden="true" />
            {data.kind === 'group' ? 'Group chat analytics' : data.kind === 'annual' ? 'Annual chat analytics' : 'Chat analytics'}
          </div>
          <h1 className="max-w-3xl text-2xl font-semibold tracking-tight md:text-3xl">{data.heading}</h1>
          <p className="max-w-3xl text-sm opacity-85">{data.subtitle}</p>
          <div className="mt-2 inline-flex w-fit rounded-full bg-[var(--report-hero-fg)]/15 px-3 py-1 text-xs">
            {data.range}
          </div>
          <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs opacity-80">
            {data.meta.map((item) => (
              <span key={item}>{item}</span>
            ))}
          </div>
        </div>
      </header>

      <section
        aria-label="核心指标"
        className={cn(
          'grid grid-cols-2 gap-3 md:grid-cols-3',
          data.kpis.length >= 6
            ? 'xl:grid-cols-6'
            : data.kpis.length === 5
              ? 'lg:grid-cols-5'
              : 'md:grid-cols-4',
        )}
      >
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

      {data.kind === 'annual' && data.annual && <AnnualSections annual={data.annual} />}

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
          description="基于文本消息样本 · 字号表示出现次数"
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
