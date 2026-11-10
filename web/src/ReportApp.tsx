import { useEffect, useMemo, useRef, useState } from 'react'
import { CalendarDays, Info, MessagesSquare, Moon, Sun, Users } from 'lucide-react'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
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
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
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
  avatar?: string | null
}

export interface AnnualCompanion {
  period: string
  month: number
  status: 'ok' | 'empty' | 'future'
  conversationId: string
  displayName: string
  messages: number
  periodMessages: number
  share?: number
  avatar?: string | null
}

export interface AnnualData {
  year: number
  friendCount: number
  topFriends: AnnualPerson[]
  monthlyCompanions: AnnualCompanion[]
  heatmap: {
    weekdayHourCounts: number[][]
    peak: { weekday: number; hour: number; messages: number } | null
  }
  nightCompanion: { conversationId: string; displayName: string; nightMessages: number; share: number; avatar?: string | null } | null
  nightTotal: number
  nightShareOfAll: number | null
  initiative: {
    startsMine: number
    startsPeer: number
    rateMine: number | null
    topInitiated: { conversationId: string; displayName: string; startsMine: number; startsPeer: number; avatar?: string | null } | null
  }
  response: {
    mine: { averageMs: number; count: number } | null
    peer: { averageMs: number; count: number } | null
    fastestFriend: { conversationId: string; displayName: string; averageMs: number; replies: number; avatar?: string | null } | null
  }
  balancedFriend: { conversationId: string; displayName: string; sent: number; received: number; balance: number; avatar?: string | null } | null
  streak: { conversationId: string; displayName: string; days: number; startDate: string; endDate: string; avatar?: string | null } | null
  accountStreak: { days: number; startDate: string; endDate: string } | null
  busiestDay: { date: string; messages: number; topFriend: { conversationId?: string; displayName: string; messages: number; avatar?: string | null } | null } | null
  fading: {
    conversationId: string
    displayName: string
    earlyMessages: number
    lateMessages: number
    earlyStart?: string
    earlyEnd?: string
    lateStart?: string
    lateEnd?: string
    avatar?: string | null
  } | null
  phrases: Array<{ text: string; count: number }>
  summary: Array<{ label: string; value: string; person?: string; conversationId?: string; avatar?: string | null }>
  moments: {
    status: string
    posts: number
    likesReceived: number
    likesGiven: number
    postTypes: Array<{ contentType: string; count: number }>
    topLikers: Array<{ username: string; displayName?: string; count: number; avatar?: string | null }>
    topLikedAuthors: Array<{ username: string; displayName?: string; count: number; avatar?: string | null }>
  } | null
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

function TypeList({ types, total, showPercent = true }: { types: ReportCount[]; total: number; showPercent?: boolean }) {
  if (types.length === 0) return <ChartEmpty />
  const max = Math.max(...types.map((item) => item.count), 1)
  return (
    <div className="flex flex-col gap-3">
      {types.map((item) => (
        <div key={item.name} className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-3 text-xs">
            <span className="truncate">{item.name}</span>
            <span className="shrink-0 text-muted-foreground tabular-nums">
              {showPercent ? `${item.count} · ${Math.round(pct(item.count, total))}%` : `${formatNumber(item.count)} 条`}
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

function formatNumber(value: number): string {
  return new Intl.NumberFormat('zh-CN').format(value)
}

function numberPercent(value: number | null): string {
  return value === null ? '-' : `${Math.round(value * 100)}%`
}

function AnnualFigure({ value, unit, label, className }: { value: string; unit?: string; label?: string; className?: string }) {
  return (
    <p className={cn('annual-figure', className)}>
      <span className="annual-figure-value">
        <strong>{value}</strong>
        {unit && <small>{unit}</small>}
      </span>
      {label && <small>{label}</small>}
    </p>
  )
}

function AnnualChapter({
  id,
  index,
  title,
  question,
  hint,
  accent,
  ghost,
  children,
}: {
  id: string
  index: number
  title: string
  question: string
  hint: string
  accent: 'time' | 'people' | 'quantity'
  ghost?: string
  children: React.ReactNode
}) {
  return (
    <section className="annual-chapter" id={`annual-${id}`} data-accent={accent} aria-labelledby={`annual-${id}-question`}>
      {ghost && <span className="annual-ghost" aria-hidden="true">{ghost}</span>}
      <div className="annual-kicker">
        <span className="annual-kicker-index">{String(index).padStart(2, '0')}</span>
        <span>{title}</span>
        <Tooltip>
          <TooltipTrigger asChild>
            <button type="button" className="annual-hint" aria-label="怎么算的">
              <Info aria-hidden="true" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="top" sideOffset={8} className="annual-help-content">
            <span className="annual-help-label">怎么算的</span>
            <p>{hint}</p>
          </TooltipContent>
        </Tooltip>
      </div>
      <h2 id={`annual-${id}-question`} tabIndex={-1}>{question}</h2>
      {children}
      <p className="annual-folio" aria-hidden="true">年度报告 · {index} / 12</p>
    </section>
  )
}

function AnnualSplit({ first, second, title }: { first: { label: string; value: number }; second: { label: string; value: number }; title: string }) {
  const total = Math.max(first.value + second.value, 1)
  return (
    <figure className="annual-visual" aria-label={title}>
      <div className="annual-split">
        <span className="first" style={{ flexGrow: Math.max(first.value / total * 100, 2) }}>
          {first.value > 0 ? `${first.label} ${Math.round(first.value / total * 100)}%` : ''}
        </span>
        <span className="second" style={{ flexGrow: Math.max(second.value / total * 100, 2) }}>
          {second.value > 0 ? `${second.label} ${Math.round(second.value / total * 100)}%` : ''}
        </span>
      </div>
      <div className="flex flex-wrap justify-between gap-2 text-xs text-muted-foreground">
        <span>{first.label} · {formatNumber(first.value)}</span>
        <span>{second.label} · {formatNumber(second.value)}</span>
      </div>
    </figure>
  )
}

function annualTypeRows(types: ReportCount[]): ReportCount[] {
  const order = ['文字', '表情', '图片', '语音', '链接', '通话', '拍一拍', '其他']
  const counts = new Map<string, number>()
  for (const item of types) {
    let name = '其他'
    if (/文本|文字/.test(item.name)) name = '文字'
    else if (/表情/.test(item.name)) name = '表情'
    else if (/图片/.test(item.name)) name = '图片'
    else if (/语音/.test(item.name)) name = '语音'
    else if (/链接/.test(item.name)) name = '链接'
    else if (/通话/.test(item.name)) name = '通话'
    else if (/拍一拍/.test(item.name)) name = '拍一拍'
    counts.set(name, (counts.get(name) ?? 0) + item.count)
  }
  return order
    .map((name) => ({ name, count: counts.get(name) ?? 0 }))
    .filter((item) => item.count > 0)
    .sort((a, b) => b.count - a.count || order.indexOf(a.name) - order.indexOf(b.name))
}

function hourRange(hour: number): string {
  return `${String(hour).padStart(2, '0')}:00–${String((hour + 1) % 24).padStart(2, '0')}:00`
}

function dateFraction(value: string | undefined, endOfDay = false): number | null {
  if (!value) return null
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value)
  if (!match) return null
  const year = Number(match[1])
  const month = Number(match[2])
  const day = Number(match[3])
  if (!year || !month || !day) return null
  const daysInMonth = new Date(year, month, 0).getDate()
  const offset = endOfDay ? day / daysInMonth : (day - 1) / daysInMonth
  return (month - 1 + offset) / 12
}

function timelineStyle(start: string | undefined, end: string | undefined): React.CSSProperties {
  const left = dateFraction(start) ?? 0
  const right = dateFraction(end, true) ?? left
  return {
    left: `${Math.max(0, Math.min(1, left)) * 100}%`,
    width: `${Math.max(0.6, (Math.min(1, right) - Math.max(0, left)) * 100)}%`,
  }
}

function monthDay(value: string | undefined): string {
  if (!value) return ''
  const match = /^\d{4}-(\d{2})-(\d{2})/.exec(value)
  if (!match) return value
  return `${Number(match[1])}月${Number(match[2])}日`
}

function chineseDate(value: string | undefined): string {
  if (!value) return ''
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value)
  if (!match) return value
  return `${match[1]}年${Number(match[2])}月${Number(match[3])}日`
}

function shortDateRange(start: string | undefined, end: string | undefined): string {
  if (!start || !end) return ''
  const left = /^(\d{4})-(\d{2})-(\d{2})/.exec(start)
  const right = /^(\d{4})-(\d{2})-(\d{2})/.exec(end)
  if (!left || !right) return `${start} 至 ${end}`
  return `${left[1]}年${Number(left[2])}月${Number(left[3])}日 至 ${Number(right[2])}月${Number(right[3])}日`
}

function PersonAvatar({ person, size = 'sm', className }: { person: { displayName: string; avatar?: string | null }; size?: 'sm' | 'default' | 'lg'; className?: string }) {
  return (
    <Avatar size={size} className={cn('annual-person-avatar', className)} aria-hidden="true">
      {person.avatar && <AvatarImage src={person.avatar} alt="" />}
      <AvatarFallback>{(person.displayName || '?').slice(0, 1)}</AvatarFallback>
    </Avatar>
  )
}

function PersonName({ person }: { person: { displayName: string; avatar?: string | null } }) {
  return (
    <span className="annual-person">
      <PersonAvatar person={person} />
      <strong>{person.displayName}</strong>
    </span>
  )
}

const CLOUD_MAX_TERMS = 100
const CLOUD_MIN_SCALE = 0.3
const CLOUD_WEIGHTS = [500, 500, 600, 650, 700]
const VERTICAL_TERM = /^[\p{Script=Han}]{2,6}$/u
const HAN_LINE = /^[\p{Script=Han}\p{P}]+$/u

interface CloudTerm {
  term: string
  count: number
}

interface CloudWord extends CloudTerm {
  x: number
  y: number
  fontSize: number
  level: number
  vertical: boolean
  ink: number
}

interface CloudFrame {
  width: number
  height: number
  minFont: number
  measure: (text: string, fontSize: number, level: number) => number
  bounds: { left: number; top: number; right: number; bottom: number }
  probes: ReturnType<typeof probeRings>
}

interface CloudLayoutWord extends CloudTerm {
  scale: number
  level: number
  ink: number
  phase: number
  orientations: boolean[]
}

function cloudTerms(terms: CloudTerm[], limit = CLOUD_MAX_TERMS): CloudTerm[] {
  const merged = new Map<string, number>()
  for (const { term, count } of terms) {
    const text = term.trim()
    if (!text || !Number.isFinite(count) || count <= 0) continue
    merged.set(text, (merged.get(text) ?? 0) + count)
  }
  return [...merged]
    .map(([term, count]) => ({ term, count }))
    .sort((a, b) => b.count - a.count || (a.term < b.term ? -1 : a.term > b.term ? 1 : 0))
    .slice(0, limit)
}

function cloudRatio(count: number, min: number, max: number): number {
  if (max <= min) return 1
  return Math.max(0, Math.min(1, (Math.sqrt(count) - Math.sqrt(min)) / (Math.sqrt(max) - Math.sqrt(min))))
}

function estimateTextWidth(text: string, fontSize: number): number {
  let em = 0
  for (const char of text) {
    em += /\p{Extended_Pictographic}/u.test(char) ? 1.2 : /[\u2E80-\u9FFF\uAC00-\uD7AF\uFF00-\uFFEF]/u.test(char) ? 1 : /[A-Z0-9mwMW]/.test(char) ? 0.68 : 0.56
  }
  return Math.max(1, em) * fontSize
}

function cloudWeight(level: number): number {
  return CLOUD_WEIGHTS[Math.max(0, Math.min(CLOUD_WEIGHTS.length - 1, Math.round(level)))]
}

function cloudWordBox(term: string, fontSize: number, vertical: boolean, measure: CloudFrame['measure']) {
  if (vertical) return { width: fontSize, height: [...term].length * fontSize }
  return { width: measure(term, fontSize, 0), height: fontSize * (HAN_LINE.test(term) ? 1.02 : 1.16) }
}

function wordGap(fontSize: number): number {
  return Math.max(2, fontSize * 0.06)
}

function termHash(term: string): number {
  let hash = 2166136261
  for (let index = 0; index < term.length; index++) {
    hash ^= term.charCodeAt(index)
    hash = Math.imul(hash, 16777619)
  }
  return hash >>> 0
}

function occupancy(width: number, height: number) {
  const BUCKET = 48
  const COVER = 4
  const cols = Math.max(1, Math.ceil(width / BUCKET))
  const rows = Math.max(1, Math.ceil(height / BUCKET))
  const buckets: Array<Array<{ left: number; top: number; right: number; bottom: number }>> = Array.from({ length: cols * rows }, () => [])
  const coverCols = Math.max(1, Math.ceil(width / COVER))
  const coverRows = Math.max(1, Math.ceil(height / COVER))
  const covered = new Uint8Array(coverCols * coverRows)
  const cell = (value: number, limit: number) => Math.max(0, Math.min(limit - 1, Math.floor(value / BUCKET)))
  return {
    covered(x: number, y: number) {
      const col = Math.floor(x / COVER)
      const row = Math.floor(y / COVER)
      return col >= 0 && row >= 0 && col < coverCols && row < coverRows && covered[row * coverCols + col] === 1
    },
    free(left: number, top: number, right: number, bottom: number) {
      for (let row = cell(top, rows), lastRow = cell(bottom, rows); row <= lastRow; row++) {
        for (let col = cell(left, cols), lastCol = cell(right, cols); col <= lastCol; col++) {
          for (const other of buckets[row * cols + col]) {
            if (left < other.right && right > other.left && top < other.bottom && bottom > other.top) return false
          }
        }
      }
      return true
    },
    add(box: { left: number; top: number; right: number; bottom: number }) {
      for (let row = cell(box.top, rows), lastRow = cell(box.bottom, rows); row <= lastRow; row++) {
        for (let col = cell(box.left, cols), lastCol = cell(box.right, cols); col <= lastCol; col++) buckets[row * cols + col].push(box)
      }
      const lastCol = Math.min(coverCols, Math.floor(box.right / COVER))
      const lastRow = Math.min(coverRows, Math.floor(box.bottom / COVER))
      for (let row = Math.max(0, Math.ceil(box.top / COVER)); row < lastRow; row++) {
        for (let col = Math.max(0, Math.ceil(box.left / COVER)); col < lastCol; col++) covered[row * coverCols + col] = 1
      }
    },
  }
}

function probeRings(cx: number, cy: number, halfWidth: number, halfHeight: number) {
  const RING_STEP = 4
  const PROBE_STEP = 4
  const rings = Math.max(1, Math.ceil(Math.min(halfWidth, halfHeight) / RING_STEP))
  const xs = [cx]
  const ys = [cy]
  const starts = [0]
  for (let ring = 1; ring <= rings; ring++) {
    starts.push(xs.length)
    const a = (halfWidth * ring) / rings
    const b = (halfHeight * ring) / rings
    const perimeter = 4 * (a + b)
    const count = Math.max(8, Math.ceil(perimeter / PROBE_STEP))
    for (let index = 0; index < count; index++) {
      let t = (index / count) * perimeter
      if (t < 2 * a) {
        xs.push(cx - a + t)
        ys.push(cy - b)
        continue
      }
      t -= 2 * a
      if (t < 2 * b) {
        xs.push(cx + a)
        ys.push(cy - b + t)
        continue
      }
      t -= 2 * b
      if (t < 2 * a) {
        xs.push(cx + a - t)
        ys.push(cy + b)
        continue
      }
      xs.push(cx - a)
      ys.push(cy + b - (t - 2 * a))
    }
  }
  starts.push(xs.length)
  return {
    x: Float64Array.from(xs),
    y: Float64Array.from(ys),
    ringStart: Int32Array.from(starts),
    halfWidth,
    halfHeight,
    rings,
  }
}

function findSpot(
  frame: CloudFrame,
  occupied: ReturnType<typeof occupancy>,
  halfW: number,
  halfH: number,
  gap: number,
  phase: number,
) {
  const { probes, bounds } = frame
  const reachX = probes.halfWidth - halfW
  const reachY = probes.halfHeight - halfH
  if (reachX < 0 || reachY < 0) return null
  for (let ring = 0; ring <= probes.rings; ring++) {
    if ((probes.halfWidth * ring) / probes.rings > reachX && (probes.halfHeight * ring) / probes.rings > reachY) return null
    const start = probes.ringStart[ring]
    const count = probes.ringStart[ring + 1] - start
    const offset = Math.floor(phase * count)
    for (let step = 0; step < count; step++) {
      const index = start + ((offset + step) % count)
      const x = probes.x[index]
      const y = probes.y[index]
      if (x - halfW < bounds.left || x + halfW > bounds.right || y - halfH < bounds.top || y + halfH > bounds.bottom || occupied.covered(x, y)) continue
      if (occupied.free(x - halfW - gap, y - halfH - gap, x + halfW + gap, y + halfH + gap)) {
        return { left: x - halfW - gap, top: y - halfH - gap, right: x + halfW + gap, bottom: y + halfH + gap }
      }
    }
  }
  return null
}

function placeWords(words: CloudLayoutWord[], top: number, frame: CloudFrame, strict: boolean): CloudWord[] | null {
  const occupied = occupancy(frame.width, frame.height)
  const placed: CloudWord[] = []
  for (const word of words) {
    const fontSize = Math.max(frame.minFont, Math.round(top * word.scale))
    const gap = wordGap(fontSize)
    let spot = null
    let vertical = false
    for (const orientation of word.orientations) {
      const box = cloudWordBox(word.term, fontSize, orientation, frame.measure)
      spot = findSpot(frame, occupied, box.width / 2, box.height / 2, gap, word.phase)
      if (spot) {
        vertical = orientation
        break
      }
    }
    if (!spot) {
      if (strict) return null
      continue
    }
    occupied.add(spot)
    placed.push({
      term: word.term,
      count: word.count,
      x: Math.round(((spot.left + spot.right) / 2) * 10) / 10,
      y: Math.round(((spot.top + spot.bottom) / 2) * 10) / 10,
      fontSize,
      level: word.level,
      vertical,
      ink: word.ink,
    })
  }
  return placed
}

function layoutWordCloud(terms: CloudTerm[], width: number, height: number, measure: CloudFrame['measure']): CloudWord[] {
  const minFont = 11
  const inset = 10
  const items = cloudTerms(terms)
  const innerWidth = width - 2 * inset
  const innerHeight = height - 2 * inset
  if (!items.length || innerWidth <= 0 || innerHeight <= 0) return []
  const max = items[0].count
  const min = items[items.length - 1].count
  const words: CloudLayoutWord[] = items.map((item, rank) => {
    const hash = termHash(item.term)
    const ratio = max === min ? 1 : (item.count - min) / (max - min)
    const standing = rank >= 3 && VERTICAL_TERM.test(item.term)
    const upright = standing && (hash >>> 8) % 10 < 3
    return {
      ...item,
      scale: CLOUD_MIN_SCALE + (1 - CLOUD_MIN_SCALE) * cloudRatio(item.count, min, max),
      level: Math.max(1, Math.ceil(Math.sqrt(ratio) * 4)),
      ink: rank === 0 ? 0 : hash % 4,
      phase: ((hash >>> 16) % 997) / 997,
      orientations: !standing ? [false] : upright ? [true, false] : [false, true],
    }
  })
  const frame: CloudFrame = {
    width,
    height,
    minFont,
    measure,
    bounds: { left: inset, top: inset, right: width - inset, bottom: height - inset },
    probes: probeRings(width / 2, height / 2, innerWidth / 2, innerHeight / 2),
  }
  const perPixel = Math.max(0.5, measure(words[0].term, 100, words[0].level) / 100)
  const cap = Math.min(innerHeight * 0.3, 128)
  let ceiling = Math.max(minFont, Math.min(cap, (innerWidth - 8) / perPixel, innerHeight / 1.3))
  const inkArea = (top: number) => words.reduce((sum, word) => {
    const box = cloudWordBox(word.term, Math.max(minFont, Math.round(top * word.scale)), false, measure)
    return sum + box.width * box.height
  }, 0)
  const packable = 0.72 * innerWidth * innerHeight
  if (inkArea(ceiling) > packable) {
    let low = minFont
    let high = ceiling
    while (high - low > 1) {
      const middle = (low + high) / 2
      if (inkArea(middle) > packable) high = middle
      else low = middle
    }
    ceiling = low
  }
  const fits = (top: number) => placeWords(words, top, frame, true)
  let best = fits(ceiling)
  if (best) return best
  best = fits(minFont)
  if (!best) return placeWords(words, minFont, frame, false) ?? []
  let low = minFont
  let high = ceiling
  while (high - low > 1) {
    const middle = (low + high) / 2
    const placed = fits(middle)
    if (placed) {
      low = middle
      best = placed
    } else high = middle
  }
  return best ?? []
}

function makeCloudMeasure(): CloudFrame['measure'] {
  const context = document.createElement('canvas').getContext('2d')
  if (!context) return (text, fontSize) => estimateTextWidth(text, fontSize)
  const widths = new Map<string, number>()
  return (text, fontSize, level) => {
    const weight = cloudWeight(level)
    const key = `${weight}:${text}`
    let width = widths.get(key)
    if (width === undefined) {
      context.font = `${weight} 100px "Noto Sans SC", "PingFang SC", "Microsoft YaHei", sans-serif`
      width = context.measureText(text).width
      widths.set(key, width)
    }
    return (width * fontSize) / 100
  }
}

function useCloudSize(ref: React.RefObject<HTMLDivElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 })
  useEffect(() => {
    const element = ref.current
    if (!element) return
    const apply = () => {
      const rect = element.getBoundingClientRect()
      setSize((current) => current.width === Math.floor(rect.width) && current.height === Math.floor(rect.height)
        ? current
        : { width: Math.floor(rect.width), height: Math.floor(rect.height) })
    }
    apply()
    const observer = new ResizeObserver(apply)
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref])
  return size
}

function PhraseWordCloud({ phrases }: { phrases: CloudTerm[] }) {
  const container = useRef<HTMLDivElement>(null)
  const size = useCloudSize(container)
  const measure = useMemo(() => makeCloudMeasure(), [])
  const words = useMemo(
    () => size.width > 0 && size.height > 0 ? layoutWordCloud(phrases, size.width, size.height, measure) : [],
    [phrases, size.width, size.height, measure],
  )
  const [hover, setHover] = useState<CloudWord | null>(null)

  return (
    <div ref={container} className="annual-wordcloud-chart" aria-label="口头禅词云">
      {size.width > 0 && (
        <svg className="annual-wordcloud-svg" viewBox={`0 0 ${size.width} ${size.height}`} role="img" aria-label="口头禅词云">
          {words.map((word) => {
            const characters = [...word.term]
            const first = word.y - ((characters.length - 1) * word.fontSize) / 2
            return (
              <g
                key={word.term}
                className="annual-cloud-word"
                data-ink={word.ink}
                data-level={word.level}
                tabIndex={0}
                role="button"
                aria-label={`第 ${words.indexOf(word) + 1} 名，${word.term}，${word.count} 次`}
                onMouseEnter={() => setHover(word)}
                onMouseLeave={() => setHover(null)}
                onFocus={() => setHover(word)}
                onBlur={() => setHover(null)}
              >
                {word.vertical ? (
                  <text x={word.x} y={first} textAnchor="middle" dominantBaseline="central" fontSize={word.fontSize} fontWeight={cloudWeight(word.level)}>
                    {characters.map((character, index) => (
                      <tspan key={index} x={word.x} y={first + index * word.fontSize} dominantBaseline="central">{character}</tspan>
                    ))}
                  </text>
                ) : (
                  <text x={word.x} y={word.y} textAnchor="middle" dominantBaseline="central" fontSize={word.fontSize} fontWeight={cloudWeight(word.level)}>
                    {word.term}
                  </text>
                )}
              </g>
            )
          })}
        </svg>
      )}
      {hover && (
        <div
          className="annual-cloud-hover"
          style={{
            left: Math.min(Math.max(hover.x, 70), Math.max(70, size.width - 70)),
            top: Math.max(hover.y - hover.fontSize / 2 - 4, 0),
          }}
        >
          <div className="annual-cloud-tooltip">
            <span className="annual-cloud-tooltip-rank">第 {words.indexOf(hover) + 1} 名</span>
            <span className="annual-cloud-tooltip-word">[{hover.term}]</span>
            <span className="annual-cloud-tooltip-line" />
            <span className="annual-cloud-tooltip-count">{hover.count} 次</span>
          </div>
        </div>
      )}
    </div>
  )
}

function PhraseTooltip({ rank, phrase }: { rank: number; phrase: { text: string; count: number } }) {
  return (
    <TooltipContent side="top" sideOffset={7} className="annual-phrase-tooltip">
      <span className="annual-phrase-tooltip-rank">第 {rank} 名</span>
      <span className="annual-phrase-tooltip-word">[{phrase.text}]</span>
      <span className="annual-phrase-tooltip-line" aria-hidden="true" />
      <span className="annual-phrase-tooltip-count">{phrase.count} 次</span>
    </TooltipContent>
  )
}

function FriendName({ name }: { name: string }) {
  return <strong className="annual-friend-name">{name}</strong>
}

function AnnualReportView({
  data,
  annual,
  dark,
  setDark,
}: {
  data: ReportData
  annual: AnnualData
  dark: boolean
  setDark: (update: (value: boolean) => boolean) => void
}) {
  const [active, setActive] = useState('overview')
  const messageTotal = data.kpis.find((item) => item.label === '消息总数')?.value ?? '0'
  const messageTotalNumber = data.types.reduce((sum, item) => sum + item.count, 0)
  const typeRows = annualTypeRows(data.types)
  const cloudPhrases = useMemo(
    () => annual.phrases.slice(0, 30).map((item) => ({ term: item.text, count: item.count })),
    [annual.phrases],
  )
  const closingLabels = ['私聊消息', '聊得最多', '你先开口', '最长连续', '口头禅', '聊过的好友', '最常聊天', '你的平均回复', '私聊最多的一天', '朋友圈']
  const closingRows = closingLabels
    .map((label) => annual.summary.find((item) => item.label === label))
    .filter((item): item is NonNullable<typeof item> => Boolean(item))
  const [firstMessageTime, lastMessageTime] = data.range.split(' → ')
  const chapters = [
    'overview', 'top-friends', 'companions', 'rhythm', 'initiative', 'response',
    'balance', 'streak', 'fading', 'phrases', 'moments', 'closing',
  ]
  const labels = [
    '概览', '聊得最多的人', '每个月陪你最多的人', '什么时候最常聊天', '谁先开口', '回得多快',
    '最有来有往的人', '连续与高峰', '渐渐少聊的人', '你的口头禅', '朋友圈', '尾声',
  ]

  useEffect(() => {
    const sections = chapters
      .map((id) => document.getElementById(`annual-${id}`))
      .filter((section): section is HTMLElement => section !== null)
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            setActive(entry.target.id.replace('annual-', ''))
          }
        }
      },
      { rootMargin: '-20% 0px -65% 0px', threshold: 0 },
    )
    sections.forEach((section) => observer.observe(section))
    return () => observer.disconnect()
  }, [])

  const go = (id: string) => {
    document.getElementById(`annual-${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    setActive(id)
  }

  const top = annual.topFriends[0]
  const topShare = top && messageTotalNumber > 0 ? Math.round((top.messages / messageTotalNumber) * 100) : 0
  const activeMonths = annual.monthlyCompanions.filter((item) => item.status === 'ok')
  const monthlyLeaders = new Map<string, { item: AnnualCompanion; months: number; messages: number }>()
  for (const item of activeMonths) {
    if (!item.conversationId) continue
    const current = monthlyLeaders.get(item.conversationId)
    if (current) {
      current.months += 1
      current.messages += item.messages
    } else {
      monthlyLeaders.set(item.conversationId, { item, months: 1, messages: item.messages })
    }
  }
  const monthlyLeader = [...monthlyLeaders.values()].sort((a, b) => b.months - a.months || b.messages - a.messages)[0]
  const otherMonthlyLeaders = Math.max(0, monthlyLeaders.size - (monthlyLeader ? 1 : 0))
  const podium = [annual.topFriends[1], annual.topFriends[0], annual.topFriends[2]].filter(Boolean) as AnnualPerson[]
  const runnersUp = annual.topFriends.slice(3, 10)
  const peak = annual.heatmap.peak
  const night = annual.nightCompanion
  const balance = annual.balancedFriend
  const streak = annual.streak
  const busiest = annual.busiestDay
  const fading = annual.fading
  const phrase = annual.phrases[0]

  return (
    <div className="annual-report">
      <div className="annual-toolbar">
        <div className="annual-toolbar-title">
          <strong>年度报告</strong>
          <span>{data.heading} · {data.subtitle}</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="hidden text-xs text-muted-foreground sm:inline">{data.range}</span>
          <Button
            variant="outline"
            size="icon"
            aria-label={dark ? '切换到浅色主题' : '切换到深色主题'}
            onClick={() => setDark((value) => !value)}
          >
            {dark ? <Sun data-icon="inline-start" aria-hidden="true" /> : <Moon data-icon="inline-start" aria-hidden="true" />}
          </Button>
        </div>
      </div>

      <div className="annual-wrap">
        <AnnualChapter
          id="overview"
          index={1}
          title="概览"
          question={annual.topFriends.length > 0 ? '这段时间，你的私聊都发生了什么？' : '这段时间，你的私聊都发生了什么？'}
          hint="只算你和好友一对一的私聊，你发的和收到的都算；群聊、公众号、服务号、文件传输助手和系统提示都不算。"
          accent="quantity"
          ghost={data.range.slice(0, 4)}
        >
          <AnnualFigure value={messageTotal} unit="条" label={`私聊消息`} />
          <div className="annual-lead">
            <p>
              <strong>{annual.year > 0 ? `${annual.year} 年` : data.heading.match(/\d{4}/)?.[0] ? `${data.heading.match(/\d{4}/)![0]} 年` : '这些年'}</strong>，你和{' '}
              <strong>{annual.friendCount ?? annual.topFriends.length}</strong> 位好友在私聊里收发了{' '}
              <strong>{messageTotal}</strong> 条消息。
            </p>
            <p>
              其中你发出 <strong>{formatNumber(data.ratio?.me ?? 0)}</strong> 条，收到{' '}
              <strong>{formatNumber(data.ratio?.other ?? 0)}</strong> 条。
            </p>
          </div>
          <div className="annual-stats">
            {data.kpis.slice(0, 5).map((kpi) => (
              <div className="annual-stat" key={kpi.label}>
                <span>{kpi.label}</span>
                <strong>{kpi.value}</strong>
                <small>{kpi.note}</small>
              </div>
            ))}
          </div>
          <div className="annual-overview-details">
            <div className="annual-times">
              <div>
                <span>第一条消息</span>
                <strong>{firstMessageTime || '-'}</strong>
              </div>
              <div>
                <span>最后一条消息</span>
                <strong>{lastMessageTime || '-'}</strong>
              </div>
            </div>
            <div className="annual-types">
              <h3>消息类型</h3>
              <TypeList types={typeRows} total={messageTotalNumber} showPercent={false} />
            </div>
          </div>
        </AnnualChapter>

        <AnnualChapter
          id="top-friends"
          index={2}
          title="聊得最多的人"
          question="这一年，你和谁聊得最多？"
          hint="按你和每位好友的私聊消息总数排序，你发的和 TA 发的都算。"
          accent="people"
          ghost=""
        >
          {top ? (
            <>
              <div className="annual-top-hero">
                <div className="annual-top-avatar">
                  <Avatar className="size-full">
                    {top.avatar && <AvatarImage src={top.avatar} alt="" />}
                    <AvatarFallback>{top.displayName.slice(0, 1)}</AvatarFallback>
                  </Avatar>
                </div>
                <div className="annual-top-copy">
                  <span>这一年和你聊得最多</span>
                  <strong>{top.displayName}</strong>
                  <small>{formatNumber(top.messages)} 条消息</small>
                </div>
              </div>
              <div className="annual-lead">
                <p>
                  你和 <strong>{top.displayName}</strong> 聊得最多：一共{' '}
                  <strong>{formatNumber(top.messages)}</strong> 条，其中你发了 <strong>{formatNumber(top.sent ?? 0)}</strong> 条，TA 发了{' '}
                  <strong>{formatNumber(top.received ?? 0)}</strong> 条，占你全部私聊消息的 <strong>{topShare}%</strong>。
                </p>
              </div>
              <ol className="annual-podium" aria-label="聊天最多的前三名">
                {podium.map((friend) => {
                  const place = annual.topFriends.indexOf(friend) + 1
                  return (
                    <li className={`annual-podium-place place-${place}`} key={friend.conversationId}>
                      <div className="annual-podium-avatar">
                        <Avatar className="size-full">
                          {friend.avatar && <AvatarImage src={friend.avatar} alt="" />}
                          <AvatarFallback>{friend.displayName.slice(0, 1)}</AvatarFallback>
                        </Avatar>
                      </div>
                      <span className="annual-podium-name">{friend.displayName}</span>
                      <span className="annual-podium-messages">{formatNumber(friend.messages)} 条</span>
                      <div
                        className="annual-podium-bar"
                        role="img"
                        aria-label={`我发 ${friend.sent ?? 0} 条，TA 发 ${friend.received ?? 0} 条`}
                      >
                        <span className="me" style={{ width: `${(friend.sent ?? 0) / Math.max((friend.sent ?? 0) + (friend.received ?? 0), 1) * 100}%` }} />
                        <span className="ta" style={{ width: `${(friend.received ?? 0) / Math.max((friend.sent ?? 0) + (friend.received ?? 0), 1) * 100}%` }} />
                      </div>
                      <span className="annual-podium-caption">我发 {formatNumber(friend.sent ?? 0)} · TA 发 {formatNumber(friend.received ?? 0)}</span>
                      <div className="annual-podium-stand" aria-label={`第 ${place} 名`}>
                        <span>{place}</span>
                      </div>
                    </li>
                  )
                })}
              </ol>
              {runnersUp.length > 0 && (
                <ol className="annual-list annual-runners-up">
                  {runnersUp.map((friend, index) => (
                    <li key={friend.conversationId}>
                      <span className="rank">{String(index + 4).padStart(2, '0')}</span>
                      <PersonAvatar person={friend} />
                      <span className="name">{friend.displayName}</span>
                      <span className="value">{formatNumber(friend.messages)} 条 · 我 {friend.sent ?? 0} / TA {friend.received ?? 0}</span>
                    </li>
                  ))}
                </ol>
              )}
            </>
          ) : <p className="annual-missing">这段时间没有和好友的私聊消息。</p>}
        </AnnualChapter>

        <AnnualChapter
          id="companions"
          index={3}
          title="每个月陪你最多的人"
          question="每个月陪你聊得最多的是谁？"
          hint="每个月里和你私聊消息最多的好友；占比是 TA 的消息数除以当月全部私聊消息数。"
          accent="people"
          ghost=""
        >
          {annual.monthlyCompanions.length > 0 ? (
            <>
              {monthlyLeader && (
                <div className="annual-month-hero">
                  <PersonAvatar person={monthlyLeader.item} size="lg" className="annual-month-hero-avatar" />
                  <div className="annual-month-hero-copy">
                    <strong>{monthlyLeader.item.displayName}</strong>
                    <span>{monthlyLeader.months} 个月排第一</span>
                  </div>
                </div>
              )}
              <div className="annual-lead">
                <p>
                  {monthlyLeader ? (
                    <>
                      在有私聊的 <strong>{activeMonths.length}</strong> 个月里，<strong>{monthlyLeader.item.displayName}</strong> 有{' '}
                      <strong>{monthlyLeader.months}</strong> 个月是和你聊得最多的人
                      {otherMonthlyLeaders > 0 ? <>；另有 <strong>{otherMonthlyLeaders}</strong> 位好友各在某些时候排第一。</> : '。'}
                    </>
                  ) : '这段时间还没有可统计的月度私聊。'}
                </p>
              </div>
              <div className="annual-month-grid">
                {annual.monthlyCompanions.map((item, index) => {
                  const status = item.status ?? (item.periodMessages > 0 ? 'ok' : 'empty')
                  const next = annual.monthlyCompanions[index + 1]
                  const nextStatus = next?.status ?? (next && next.periodMessages > 0 ? 'ok' : 'empty')
                  const linked = status === 'ok' && nextStatus === 'ok'
                    && item.conversationId === next?.conversationId
                    && (index + 1) % 6 !== 0
                  const month = item.month ?? Number(item.period.slice(-2))
                  return (
                    <div
                      className="annual-month"
                      key={item.period}
                      data-status={status}
                      data-linked={linked ? 'true' : undefined}
                    >
                      <div className="annual-month-head"><span>{month}月</span></div>
                      {status === 'future' ? (
                        <span className="annual-month-empty">还没到</span>
                      ) : status === 'empty' ? (
                        <span className="annual-month-empty">没有私聊</span>
                      ) : (
                        <>
                          <div className="annual-month-person">
                            <PersonAvatar person={item} />
                            <strong>{item.displayName}</strong>
                          </div>
                          <span className="annual-month-stats">
                            {formatNumber(item.messages)} 条 · 占当月 {Math.round((item.share ?? (item.periodMessages > 0 ? item.messages / item.periodMessages : 0)) * 100)}%
                          </span>
                        </>
                      )}
                    </div>
                  )
                })}
              </div>
            </>
          ) : <p className="annual-missing">这段时间没有私聊消息。</p>}
        </AnnualChapter>

        <AnnualChapter
          id="rhythm"
          index={4}
          title="什么时候最常聊天"
          question="你通常在什么时候聊天？"
          hint="按消息时间所在的星期和小时累计，你发的和收到的都算；深夜指 00:00–05:59。"
          accent="time"
          ghost=""
        >
          {peak ? (
            <AnnualFigure
              className="annual-figure--time"
              value={`周${['一', '二', '三', '四', '五', '六', '日'][peak.weekday]} ${hourRange(peak.hour)}`}
              label={`最常聊天的时段 · ${peak.messages} 条`}
            />
          ) : null}
          <div className="annual-lead">
            <p>
              {peak ? `你最常在周${['一', '二', '三', '四', '五', '六', '日'][peak.weekday]} ${hourRange(peak.hour)} 聊天，这个时段累计 ${peak.messages} 条消息。` : '没有可以统计时段的消息。'}
            </p>
          </div>
          <div className="annual-heat" aria-label="星期和小时热力图">
            <span />
            {Array.from({ length: 24 }, (_, hour) => (
              <span className="hour" key={hour}>{hour % 3 === 0 ? hour : ''}</span>
            ))}
            {annual.heatmap.weekdayHourCounts.map((row, weekday) => {
              const day = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'][weekday]
              const max = Math.max(...annual.heatmap.weekdayHourCounts.flat(), 1)
              return (
                <div className="contents" key={weekday}>
                  <span className="day">{day}</span>
                  {row.map((count, hour) => {
                    const share = messageTotalNumber > 0 ? (count / messageTotalNumber) * 100 : 0
                    return (
                      <Tooltip key={hour}>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            className="cell"
                            style={{ opacity: Math.max(0.08, count / max) }}
                            aria-label={`${day} ${String(hour).padStart(2, '0')}:00，${count} 条消息，占全部 ${share.toFixed(1)}%`}
                          />
                        </TooltipTrigger>
                        <TooltipContent side="top" sideOffset={8} className="annual-heat-tooltip">
                          <span className="annual-heat-tooltip-head">{day} {String(hour).padStart(2, '0')}:00–{String(hour + 1).padStart(2, '0')}:00</span>
                          <span className="annual-heat-tooltip-row">
                            <span className="annual-heat-dot" aria-hidden="true" />
                            消息
                            <span className="annual-heat-leader" aria-hidden="true" />
                            <strong>{formatNumber(count)} 条</strong>
                          </span>
                          <span className="annual-heat-tooltip-row">
                            占全部
                            <span className="annual-heat-leader" aria-hidden="true" />
                            <strong>{share.toFixed(1)}%</strong>
                          </span>
                        </TooltipContent>
                      </Tooltip>
                    )
                  })}
                </div>
              )
            })}
          </div>
          <div className="annual-heat-legend" aria-label="消息量图例">
            <span>少</span>
            <span className="annual-heat-swatches" aria-hidden="true">
              {[0.16, 0.32, 0.5, 0.72, 1].map((opacity) => <i key={opacity} style={{ opacity }} />)}
            </span>
            <span>多</span>
          </div>
          {night && (
            <div className="annual-night-card">
              <Moon className="annual-night-icon" aria-hidden="true" />
              <PersonAvatar person={night} className="annual-night-avatar" />
              <strong className="annual-night-name">{night.displayName}</strong>
              <p>
                深夜（00:00–05:59）陪你聊天最多的是 <FriendName name={night.displayName} />：
                <strong>{formatNumber(night.nightMessages)}</strong> 条，占所有深夜私聊的{' '}
                <strong>{Math.round(night.share * 100)}%</strong>。你的私聊里，深夜消息一共{' '}
                <strong>{formatNumber(annual.nightTotal ?? (night.share > 0 ? Math.round(night.nightMessages / night.share) : 0))}</strong> 条，占全部的 <strong>{numberPercent(annual.nightShareOfAll)}</strong>。
              </p>
            </div>
          )}
        </AnnualChapter>

        <AnnualChapter
          id="initiative"
          index={5}
          title="谁先开口"
          question="聊天通常是谁先开口？"
          hint="隔了 1 小时以上的第一条消息，算作一段对话的开始；这条消息是谁发的，就算谁先开口。"
          accent="people"
          ghost=""
        >
          <AnnualFigure value={numberPercent(annual.initiative.rateMine)} label="的对话由你开启" />
          <div className="annual-lead">
            <p>
              你开启了 <strong>{annual.initiative.startsMine}</strong> 段对话，好友开启了{' '}
              <strong>{annual.initiative.startsPeer}</strong> 段。
              {annual.initiative.topInitiated && <>你最常主动找的是 <PersonName person={annual.initiative.topInitiated} />：你先开口 {annual.initiative.topInitiated.startsMine} 次，TA 先开口 {annual.initiative.topInitiated.startsPeer} 次。</>}
            </p>
          </div>
          <AnnualSplit
            first={{ label: '你先开口', value: annual.initiative.startsMine }}
            second={{ label: '好友先开口', value: annual.initiative.startsPeer }}
            title="谁先开口"
          />
        </AnnualChapter>

        <AnnualChapter
          id="response"
          index={6}
          title="回得多快"
          question="你回消息有多快？"
          hint="对方发来消息后，你紧接着发的下一条算一次回复，间隔不超过 24 小时；最快的好友只看你回复过至少 10 次的人。"
          accent="time"
          ghost=""
        >
          <AnnualFigure value={annual.response.mine ? formatDuration(annual.response.mine.averageMs) : '-'} label="你的平均回复用时" />
          <div className="annual-lead">
            <p>
              {annual.response.mine ? `你回好友平均用时 ${formatDuration(annual.response.mine.averageMs)}（${annual.response.mine.count} 次）。` : '没有你回复好友的记录。'}
              {annual.response.peer ? `好友回你平均用时 ${formatDuration(annual.response.peer.averageMs)}。` : ''}
            </p>
            {annual.response.fastestFriend && <p>你回 <PersonName person={annual.response.fastestFriend} /> 最快：平均 {formatDuration(annual.response.fastestFriend.averageMs)}，共 {annual.response.fastestFriend.replies} 次。</p>}
          </div>
          <div className="annual-stats">
            <div className="annual-stat"><span>你回好友</span><strong>{annual.response.mine ? formatDuration(annual.response.mine.averageMs) : '-'}</strong><small>{annual.response.mine?.count ?? 0} 次</small></div>
            <div className="annual-stat"><span>好友回你</span><strong>{annual.response.peer ? formatDuration(annual.response.peer.averageMs) : '-'}</strong><small>{annual.response.peer?.count ?? 0} 次</small></div>
            {annual.response.fastestFriend && <div className="annual-stat"><span>最快回复</span><strong>{annual.response.fastestFriend.displayName}</strong><small>{formatDuration(annual.response.fastestFriend.averageMs)}</small></div>}
          </div>
        </AnnualChapter>

        <AnnualChapter
          id="balance"
          index={7}
          title="最有来有往的人"
          question="和谁聊得最有来有往？"
          hint="只看双方都发过 50 条以上的好友，取收发最接近的一位：较少一方 ÷ 较多一方，越接近 100% 越平衡。"
          accent="people"
          ghost=""
        >
          {balance ? (() => {
            const total = balance.sent + balance.received
            const mineShare = total > 0 ? Math.round((balance.sent / total) * 100) : 0
            const otherShare = total > 0 ? 100 - mineShare : 0
            return (
              <div className="annual-balance">
                <div className="annual-balance-person">
                  <PersonAvatar person={balance} size="lg" className="annual-balance-avatar" />
                  <strong>{balance.displayName}</strong>
                </div>
                <p className="annual-balance-copy">
                  你给 <FriendName name={balance.displayName} /> 发了 <strong>{formatNumber(balance.sent)}</strong> 条，TA 给你发了{' '}
                  <strong>{formatNumber(balance.received)}</strong> 条，较少的一方是较多一方的{' '}
                  <strong>{Math.round(balance.balance * 100)}%</strong>，是收发最接近的一位。
                </p>
                <div className="annual-balance-legend">
                  <span><i className="me" aria-hidden="true" />你发 {mineShare}% · {formatNumber(balance.sent)} 条</span>
                  <span><i className="ta" aria-hidden="true" />TA 发 {otherShare}% · {formatNumber(balance.received)} 条</span>
                </div>
                <div className="annual-balance-bar" role="img" aria-label={`你发 ${balance.sent} 条，TA 发 ${balance.received} 条`}>
                  <span className="me" style={{ width: `${mineShare}%` }} />
                  <span className="ta" style={{ width: `${otherShare}%` }} />
                </div>
              </div>
            )
          })() : <p className="annual-missing">还没有双方都发过 50 条以上消息的好友。</p>}
        </AnnualChapter>

        <AnnualChapter
          id="streak"
          index={8}
          title="连续与高峰"
          question="最长连续聊了多少天？"
          hint="每天至少有一条消息，谁发的都算，至少 2 天才算。"
          accent="time"
          ghost=""
        >
          {streak ? <AnnualFigure value={String(streak.days)} unit="天" label={`和 ${streak.displayName} 连续聊天`} /> : <AnnualFigure value="-" label="没有连续 2 天以上的好友" />}
          <div className="annual-lead">
            {streak && <p>你和 <PersonName person={streak} /> 从 {streak.startDate} 到 {streak.endDate}，连续 {streak.days} 天每天都有聊天。</p>}
            {annual.accountStreak && <p>你自己最长连续 {annual.accountStreak.days} 天，从 {annual.accountStreak.startDate} 到 {annual.accountStreak.endDate}，每天都至少和一位好友聊过。</p>}
            {busiest && <p>{busiest.date} 是你私聊最多的一天，共 {busiest.messages} 条{busiest.topFriend ? <>，聊得最多的是 <PersonName person={busiest.topFriend} />，共 {busiest.topFriend.messages} 条</> : ''}。</p>}
          </div>
          {(streak || annual.accountStreak) && (
            <div className="annual-timeline" aria-label="连续聊天时间线">
              <div className="annual-timeline-months" aria-hidden="true">
                {Array.from({ length: 12 }, (_, index) => <span key={index}>{index + 1}月</span>)}
              </div>
              {streak && (
                <div className="annual-timeline-row">
                  <div className="annual-timeline-label">和 <strong>{streak.displayName}</strong> 连续 {streak.days} 天</div>
                  <div className="annual-timeline-track">
                    <span className="annual-timeline-bar is-streak" style={timelineStyle(streak.startDate, streak.endDate)} />
                  </div>
                </div>
              )}
              {annual.accountStreak && (
                <div className="annual-timeline-row">
                  <div className="annual-timeline-label">每天都和好友聊过 <strong>{annual.accountStreak.days}</strong> 天</div>
                  <div className="annual-timeline-track">
                    <span className="annual-timeline-bar is-account" style={timelineStyle(annual.accountStreak.startDate, annual.accountStreak.endDate)} />
                  </div>
                </div>
              )}
              {busiest && (
                <div className="annual-timeline-foot">
                  <span>{monthDay(busiest.date)} · {formatNumber(busiest.messages)} 条</span>
                  <i aria-hidden="true" />
                </div>
              )}
            </div>
          )}
        </AnnualChapter>

        <AnnualChapter
          id="fading"
          index={9}
          title="渐渐少聊的人"
          question="有没有渐渐少聊的人？"
          hint="把这段时间从中间分成前后两段（不足 60 天时不比较）；前段至少 80/100 条，并且超过后段 5 倍。"
          accent="people"
          ghost=""
        >
          {fading ? (
            <div className="annual-fading">
              <div className="annual-fading-person">
                <PersonAvatar person={fading} size="lg" className="annual-fading-avatar" />
                <strong>{fading.displayName}</strong>
              </div>
              <p className="annual-fading-copy">
                {chineseDate(fading.earlyStart)} 至 {chineseDate(fading.earlyEnd)}，你和 <FriendName name={fading.displayName} /> 聊了{' '}
                <strong>{formatNumber(fading.earlyMessages)}</strong> 条；{chineseDate(fading.lateStart)} 至 {chineseDate(fading.lateEnd)}，只聊了{' '}
                <strong>{formatNumber(fading.lateMessages)}</strong> 条。<span>想念的话，可以打个招呼。</span>
              </p>
              <div className="annual-fading-compare">
                <div className="annual-fading-side">
                  <span className="annual-fading-label">前段</span>
                  <div className="annual-fading-value"><strong>{formatNumber(fading.earlyMessages)}</strong><small>条</small></div>
                  <div className="annual-fading-track"><span className="early" style={{ width: '100%' }} /></div>
                  <span className="annual-fading-dates">{shortDateRange(fading.earlyStart, fading.earlyEnd)}</span>
                </div>
                <span className="annual-fading-arrow" aria-hidden="true">→</span>
                <div className="annual-fading-side">
                  <span className="annual-fading-label">后段</span>
                  <div className="annual-fading-value"><strong>{formatNumber(fading.lateMessages)}</strong><small>条</small></div>
                  <div className="annual-fading-track">
                    <span className="late" style={{ width: `${Math.max(2, fading.earlyMessages > 0 ? (fading.lateMessages / fading.earlyMessages) * 100 : 0)}%` }} />
                  </div>
                  <span className="annual-fading-dates">{shortDateRange(fading.lateStart, fading.lateEnd)}</span>
                </div>
              </div>
            </div>
          ) : <p className="annual-missing">没有发现前后两段聊天数相差很大的好友。</p>}
        </AnnualChapter>

        <AnnualChapter
          id="phrases"
          index={10}
          title="你的口头禅"
          question="这一年你最常说的话是什么？"
          hint="只看你发出的文字消息，整条消息完全相同才算同一句；2–20 个字，至少出现 2 次。"
          accent="quantity"
          ghost=""
        >
          {phrase ? (
            <>
              <AnnualFigure value={`「${phrase.text}」`} unit={`${formatNumber(phrase.count)}次`} label="最常说的短句" /> 
              <p className="annual-phrase-lead">
                你发出的消息里，<strong>「{phrase.text}」</strong>出现了 <strong>{formatNumber(phrase.count)}</strong> 次
                {annual.phrases[1] && <>，其次是<strong>「{annual.phrases[1].text}」</strong>（<strong>{formatNumber(annual.phrases[1].count)}</strong> 次）</>}。
              </p>
              <div className="annual-phrase-layout">
              <ol className="annual-phrase-list" aria-label="口头禅前 10 名">
                {annual.phrases.slice(0, 10).map((item, index) => (
                  <li key={item.text}>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <button type="button" className="annual-phrase-row">
                          <span className="rank">{index + 1}</span>
                          <span className="text">{item.text}</span>
                          <span className="leader" aria-hidden="true" />
                          <span className="count">{item.count} 次</span>
                        </button>
                      </TooltipTrigger>
                      <PhraseTooltip rank={index + 1} phrase={item} />
                    </Tooltip>
                  </li>
                ))}
              </ol>
              <div className="annual-phrase-cloud">
                <PhraseWordCloud phrases={cloudPhrases} />
              </div>
            </div>
            </>
          ) : <p className="annual-missing">没有重复出现 2 次以上的短句。</p>}
        </AnnualChapter>

        <AnnualChapter
          id="moments"
          index={11}
          title="朋友圈"
          question="你在朋友圈留下了什么？"
          hint="按发布时间统计；收到的赞只算你自己的朋友圈，点出的赞只算别人的朋友圈。"
          accent="people"
          ghost=""
        >
          {annual.moments ? (
            <>
              <AnnualFigure value={formatNumber(annual.moments.posts)} unit="条" label="你发的朋友圈" />
              <div className="annual-lead">
                <p>
                  你发了 {formatNumber(annual.moments.posts)} 条朋友圈，收到{' '}
                  {formatNumber(annual.moments.likesReceived)} 个赞；你给别人点了{' '}
                  {formatNumber(annual.moments.likesGiven)} 个赞。
                </p>
              </div>
              <div className="annual-moments-grid">
                <div className="annual-moments-types">
                  <h3>发了什么</h3>
                  {annual.moments.postTypes.length > 0 ? (
                    annual.moments.postTypes.map((item) => {
                      const max = Math.max(...annual.moments!.postTypes.map((row) => row.count), 1)
                      return (
                        <div className="annual-moment-type" key={item.contentType}>
                          <span className="label">{item.contentType}</span>
                          <div className="track"><span style={{ width: `${(item.count / max) * 100}%` }} /></div>
                          <strong>{formatNumber(item.count)}</strong>
                        </div>
                      )
                    })
                  ) : <p className="annual-missing">这段时间没有发朋友圈。</p>}
                </div>
                <div className="annual-moments-rankings">
                  <div>
                    <h3>给你点赞最多</h3>
                    {annual.moments.topLikers.length > 0 ? (
                      <ol className="annual-list">
                        {annual.moments.topLikers.slice(0, 10).map((item, index) => (
                          <li key={item.username}>
                            <span className="rank">{String(index + 1).padStart(2, '0')}</span>
                            <PersonAvatar person={{ displayName: item.displayName ?? item.username, avatar: item.avatar }} className="annual-moment-avatar" />
                            <span className="name">{item.displayName ?? item.username}</span>
                            <span className="value">{item.count} 个赞</span>
                          </li>
                        ))}
                      </ol>
                    ) : <p className="annual-missing">还没有人给你的朋友圈点赞。</p>}
                  </div>
                  <div>
                    <h3>你点赞最多</h3>
                    {annual.moments.topLikedAuthors.length > 0 ? (
                      <ol className="annual-list">
                        {annual.moments.topLikedAuthors.slice(0, 10).map((item, index) => (
                          <li key={item.username}>
                            <span className="rank">{String(index + 1).padStart(2, '0')}</span>
                            <PersonAvatar person={{ displayName: item.displayName ?? item.username, avatar: item.avatar }} className="annual-moment-avatar" />
                            <span className="name">{item.displayName ?? item.username}</span>
                            <span className="value">{item.count} 个赞</span>
                          </li>
                        ))}
                      </ol>
                    ) : <p className="annual-missing">你还没有给别人的朋友圈点过赞。</p>}
                  </div>
                </div>
              </div>
            </>
          ) : <p className="annual-missing">当前数据源没有读取到朋友圈数据。</p>}
        </AnnualChapter>

        <AnnualChapter
          id="closing"
          index={12}
          title="尾声"
          question={annual.year > 0 ? `${annual.year} 年的私聊，一页看完` : '这些年的私聊，一页看完'}
          hint="把年度报告里最重要的数字收在一起。"
          accent="quantity"
          ghost={annual.year > 0 ? String(annual.year) : '终'}
        >
          <div className="annual-lead">
            <p>
              {annual.year > 0 ? `${annual.year} 年` : '这些年'}，你和 {annual.friendCount ?? annual.topFriends.length} 位好友留下了{' '}
              {data.kpis.find((item) => item.label === '消息总数')?.value ?? '0'} 条私聊消息。
            </p>
          </div>
          <div className="annual-closing-grid">
            {[closingRows.slice(0, 5), closingRows.slice(5, 10)].map((column, columnIndex) => (
              <div className="annual-closing-column" key={columnIndex}>
                {column.map((item) => {
                  let value = item.value
                  if (item.label === '私聊最多的一天') {
                    const match = /^(\d{4})-(\d{2})-(\d{2})\s*·\s*(.+)$/.exec(value)
                    if (match) value = `${match[1]}年${Number(match[2])}月${Number(match[3])}日 · ${match[4]}`
                  }
                  return (
                    <div className="annual-closing-row" key={item.label}>
                      <span className="annual-closing-label">{item.label}</span>
                      <span className="annual-closing-value">
                        {item.person && (
                          <PersonAvatar
                            person={{ displayName: item.person, avatar: item.avatar }}
                            className="annual-closing-avatar"
                          />
                        )}
                        {item.person && <span className="annual-closing-person">{item.person}</span>}
                        <strong>{value}</strong>
                      </span>
                    </div>
                  )
                })}
              </div>
            ))}
          </div>
        </AnnualChapter>
      </div>

      <nav className="annual-dots" aria-label="年度报告章节">
        {chapters.map((id, index) => (
          <button
            type="button"
            className="annual-dot"
            key={id}
            aria-current={active === id ? 'true' : undefined}
            onClick={() => go(id)}
            aria-label={labels[index]}
          >
            <span className="annual-dot-label">{String(index + 1).padStart(2, '0')} {labels[index]}</span>
          </button>
        ))}
      </nav>
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

  if (data.kind === 'annual' && data.annual) {
    return (
      <TooltipProvider>
        <AnnualReportView data={data} annual={data.annual} dark={dark} setDark={setDark} />
      </TooltipProvider>
    )
  }

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
