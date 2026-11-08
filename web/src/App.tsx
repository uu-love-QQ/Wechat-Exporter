import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowDown,
  FileText,
  Moon,
  Pause,
  Play,
  Search,
  SearchX,
  Sun,
  TriangleAlert,
} from 'lucide-react'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group'
import { Separator } from '@/components/ui/separator'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { cn } from '@/lib/utils'
import type { ChatData, ChatMessage, ChatMedia, MediaKind, MissingMedia } from './types'

const KIND_LABEL: Record<MediaKind, string> = {
  image: '图片',
  video: '视频',
  voice: '语音',
  emoji: '表情',
  file: '文件',
}

const DEV_DATA: ChatData = {
  title: '测试群',
  kind: '群聊',
  isGroup: true,
  id: '123456789@chatroom',
  exportedAt: '2023-12-20 00:00:00',
  messages: [
    {
      index: 1,
      sender: 'Alice',
      time: '2023-12-20 00:00:10',
      day: '2023-12-20',
      mine: false,
      content: 'hello',
      quote: null,
      media: null,
      missing: null,
    },
    {
      index: 2,
      sender: '我',
      time: '2023-12-20 00:00:20',
      day: '2023-12-20',
      mine: true,
      content: 'reply',
      quote: { sender: 'Alice', text: 'hello', target: 1 },
      media: null,
      missing: null,
    },
  ],
}

function loadData(): ChatData {
  const node = document.getElementById('chat-data')
  if (node?.textContent) {
    try {
      const parsed = JSON.parse(node.textContent) as ChatData
      if (Array.isArray(parsed.messages)) return parsed
    } catch {
      /* dev fallback */
    }
  }
  return DEV_DATA
}

const DATA = loadData()

const AVATAR_HUES = [215, 160, 275, 15, 340, 190] as const

function senderAvatarStyle(name: string): React.CSSProperties {
  let hash = 0
  for (const character of name) {
    hash = (hash * 31 + character.codePointAt(0)!) >>> 0
  }
  const hue = AVATAR_HUES[hash % AVATAR_HUES.length]
  return { backgroundColor: `hsl(${hue} 62% 42%)`, color: '#ffffff' }
}

function formatDuration(milliseconds: number): string {
  const total = Math.max(0, milliseconds) / 1000
  if (total >= 60) {
    const seconds = Math.round(total)
    return `${Math.floor(seconds / 60)} 分 ${String(seconds % 60).padStart(2, '0')} 秒`
  }
  const seconds = Math.round(total * 10) / 10
  return `${Number.isInteger(seconds) ? seconds.toFixed(0) : seconds} 秒`
}

function VoicePlayer({
  src,
  durationMs,
  mine,
  seed,
}: {
  src: string
  durationMs?: number | null
  mine: boolean
  seed: number
}) {
  const audioRef = useRef<HTMLAudioElement>(null)
  const [playing, setPlaying] = useState(false)
  const [current, setCurrent] = useState(0)
  const [loaded, setLoaded] = useState(0)

  const total = loaded > 0 ? loaded : (durationMs ?? 0) / 1000
  const progress = total > 0 ? Math.min(1, current / total) : 0
  const width = Math.round(Math.min(280, Math.max(190, 150 + total * 10)))
  const bars = Array.from({ length: 22 }, (_, index) => 28 + ((index * 37 + seed * 17) % 62))

  const toggle = () => {
    const audio = audioRef.current
    if (!audio) return
    if (audio.paused) {
      void audio.play().catch(() => setPlaying(false))
    } else {
      audio.pause()
    }
  }

  const seek = (event: React.MouseEvent<HTMLButtonElement>) => {
    const audio = audioRef.current
    const rect = event.currentTarget.getBoundingClientRect()
    if (!audio || rect.width <= 0 || total <= 0) return
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width))
    audio.currentTime = ratio * total
    setCurrent(ratio * total)
  }

  return (
    <div
      style={{ width: `${width}px`, maxWidth: '100%' }}
      className={cn(
        'mt-3 flex items-center gap-2.5 rounded-full border px-2 py-1.5',
        mine ? 'mine-border mine-surface' : 'border-border bg-muted/60',
      )}
    >
      <button
        type="button"
        onClick={toggle}
        aria-label={playing ? '暂停语音' : '播放语音'}
        className={cn(
          'grid size-9 shrink-0 place-items-center rounded-full transition-transform active:scale-95',
          mine
            ? 'bg-[var(--me-fg)] text-[var(--me-bg)]'
            : 'bg-primary text-primary-foreground',
        )}
      >
        {playing ? <Pause className="size-4" aria-hidden="true" /> : <Play className="size-4 translate-x-px" aria-hidden="true" />}
      </button>
      <button
        type="button"
        onClick={seek}
        aria-label="跳转语音播放位置"
        className="flex h-7 min-w-0 flex-1 items-center gap-[2px]"
      >
        {bars.map((height, index) => {
          const active = (index + 0.5) / bars.length <= progress
          return (
            <span
              key={index}
              style={{ height: `${height}%` }}
              className={cn(
                'min-w-0 flex-1 rounded-full',
                mine
                  ? active
                    ? 'bg-[var(--me-fg)]'
                    : 'bg-[var(--me-fg)]/35'
                  : active
                    ? 'bg-primary'
                    : 'bg-primary/25',
              )}
            />
          )
        })}
      </button>
      <span
        className={cn(
          'shrink-0 pr-1 text-xs tabular-nums',
          mine ? 'mine-muted' : 'text-muted-foreground',
        )}
      >
        {total > 0 ? formatDuration(total * 1000) : '语音'}
      </span>
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        className="hidden"
        onLoadedMetadata={(event) => {
          const value = event.currentTarget.duration
          if (Number.isFinite(value) && value > 0) setLoaded(value)
        }}
        onTimeUpdate={(event) => setCurrent(event.currentTarget.currentTime)}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => {
          setPlaying(false)
          setCurrent(0)
        }}
      />
    </div>
  )
}

function initialTheme(): boolean {
  if (typeof document === 'undefined') return false
  return document.documentElement.classList.contains('dark')
}

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  )
  useEffect(() => {
    const query = window.matchMedia('(prefers-reduced-motion: reduce)')
    const onChange = () => setReduced(query.matches)
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }, [])
  return reduced
}

function MediaBlock({ media, mine, seed }: { media: ChatMedia; mine: boolean; seed: number }) {
  const href = media.path
  if (media.kind === 'image' || media.kind === 'emoji') {
    return (
      <a href={href} target="_blank" rel="noreferrer" className="mt-3 block w-fit">
        <img
          src={href}
          alt={media.name || KIND_LABEL[media.kind]}
          loading="lazy"
          width={media.width ?? undefined}
          height={media.height ?? undefined}
          className="h-auto max-h-[420px] w-auto max-w-full rounded-xl border object-contain"
        />
      </a>
    )
  }
  if (media.kind === 'video') {
    return <video src={href} controls preload="metadata" className="mt-3 w-full max-w-md rounded-xl" />
  }
  if (media.kind === 'voice') {
    return <VoicePlayer src={href} durationMs={media.durationMs} mine={mine} seed={seed} />
  }
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className={cn(
        'mt-3 inline-flex items-center gap-2 rounded-lg border px-3 py-2 text-sm',
        mine
          ? 'mine-border mine-surface text-[var(--me-fg)]'
          : 'border-border bg-muted/50 text-foreground hover:border-primary',
      )}
    >
      <FileText className="size-4 shrink-0" aria-hidden="true" />
      <span className="break-all">{media.name || '附件'}</span>
    </a>
  )
}

function MissingMediaAlert({ missing }: { missing: MissingMedia }) {
  return (
    <Alert variant="destructive" className="mt-3">
      <TriangleAlert />
      <AlertTitle>媒体未索引</AlertTitle>
      <AlertDescription>
        {KIND_LABEL[missing.kind]}
        {missing.durationMs ? ` · ${formatDuration(missing.durationMs)}` : ''}
        {missing.name ? `：${missing.name}` : ''} 未在本地索引到，已留空
      </AlertDescription>
    </Alert>
  )
}

function MessageRow({
  message,
  flash,
  showAvatar,
  onJump,
}: {
  message: ChatMessage
  flash: boolean
  showAvatar: boolean
  onJump: (index: number) => void
}) {
  const mine = message.mine
  return (
    <div
      id={`msg-${message.index}`}
      tabIndex={-1}
      className={cn(
        'message-row flex scroll-mt-36 items-end gap-2 rounded-2xl outline-none sm:scroll-mt-24',
        mine ? 'justify-end' : 'justify-start',
        flash && 'message-flash',
      )}
    >
      {!mine &&
        (showAvatar ? (
          <Avatar size="sm" className="mb-1" aria-hidden="true" style={senderAvatarStyle(message.sender)}>
            <AvatarFallback className="bg-transparent font-medium text-current">
              {(message.sender || '?').slice(0, 1)}
            </AvatarFallback>
          </Avatar>
        ) : (
          <div className="size-6 shrink-0" aria-hidden="true" />
        ))}
      <div
        className={cn(
          'bubble-card max-w-[78%] rounded-2xl border px-4 py-3 shadow-sm transition-shadow',
          mine
            ? 'border-[var(--me-border)] bg-[var(--me-bg)] text-[var(--me-fg)]'
            : 'border-border bg-card text-card-foreground',
        )}
      >
        <div
          className={cn(
            'mb-1 flex flex-wrap items-baseline gap-x-2 gap-y-0.5 text-xs',
            mine ? 'mine-muted justify-end' : 'text-muted-foreground',
          )}
        >
          <span className={cn('font-medium', mine ? 'text-[var(--me-fg)]' : 'text-foreground')}>
            {message.sender}
          </span>
          <time>{message.time}</time>
        </div>

        {message.quote && (
          <div
            className={cn(
              'mb-2 rounded-lg border-l-2 p-2 text-sm',
              mine
                ? 'mine-border mine-surface'
                : message.quote.target === null
                  ? 'border-destructive/50 bg-muted/60'
                  : 'border-primary bg-muted/60',
            )}
          >
            <div className="flex items-center gap-2 text-xs">
              <span className={cn('font-medium', mine ? 'text-[var(--me-fg)]' : 'text-foreground')}>
                {message.quote.sender || '未知'}
              </span>
              {message.quote.target !== null ? (
                <button
                  type="button"
                  onClick={() => onJump(message.quote!.target!)}
                  className={cn('ml-auto underline-offset-2 hover:underline', mine ? 'text-[var(--me-fg)]' : 'text-primary')}
                >
                  查看原消息
                </button>
              ) : (
                <span className={cn('ml-auto text-xs', mine ? 'mine-muted' : 'text-muted-foreground')}>
                  原消息不在导出范围
                </span>
              )}
            </div>
            {message.quote.target !== null ? (
              <button
                type="button"
                onClick={() => onJump(message.quote!.target!)}
                className={cn(
                  'mt-1 block w-full whitespace-pre-wrap text-left',
                  mine ? 'mine-muted' : 'text-muted-foreground',
                )}
              >
                {message.quote.text}
              </button>
            ) : (
              <div
                className={cn(
                  'mt-1 whitespace-pre-wrap',
                  mine ? 'text-[var(--me-fg)]' : 'text-foreground/90',
                )}
              >
                {message.quote.text}
              </div>
            )}
          </div>
        )}

        {message.content && !(message.media?.kind === 'voice') && (
          <div className="whitespace-pre-wrap break-words text-base leading-relaxed">{message.content}</div>
        )}

        {message.media && <MediaBlock media={message.media} mine={mine} seed={message.index} />}
        {message.missing && <MissingMediaAlert missing={message.missing} />}
      </div>
    </div>
  )
}

function App() {
  const [query, setQuery] = useState('')
  const [dark, setDark] = useState(initialTheme)
  const [flash, setFlash] = useState<number | null>(null)
  const [showBottom, setShowBottom] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const reducedMotion = usePrefersReducedMotion()

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return DATA.messages
    return DATA.messages.filter((message) => {
      const haystack = [
        message.sender,
        message.time,
        message.content,
        message.quote?.sender ?? '',
        message.quote?.text ?? '',
        message.media?.name ?? '',
        message.missing?.name ?? '',
      ]
        .join(' ')
        .toLowerCase()
      return haystack.includes(needle)
    })
  }, [query])

  useEffect(() => {
    document.title = `${DATA.title} - 聊天记录`
  }, [])

  useEffect(() => {
    document.documentElement.classList.toggle('dark', dark)
    try {
      localStorage.setItem('wechat-theme', dark ? 'dark' : 'light')
    } catch {
      /* file:// storage may be unavailable */
    }
  }, [dark])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === '/' && document.activeElement !== inputRef.current) {
        event.preventDefault()
        inputRef.current?.focus()
      }
      if (event.key === 'Escape' && document.activeElement === inputRef.current) {
        setQuery('')
        inputRef.current?.blur()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  useEffect(() => {
    const onScroll = () => setShowBottom(window.scrollY > 600)
    window.addEventListener('scroll', onScroll, { passive: true })
    onScroll()
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  const jumpTo = (index: number) => {
    const run = () => {
      const target = document.getElementById(`msg-${index}`)
      if (!target) return
      target.scrollIntoView({ behavior: reducedMotion ? 'auto' : 'smooth', block: 'start' })
      target.focus({ preventScroll: true })
      setFlash(index)
      window.setTimeout(() => setFlash((current) => (current === index ? null : current)), 1400)
    }
    if (query) {
      setQuery('')
      window.setTimeout(run, 0)
    } else {
      run()
    }
  }

  const initial = (DATA.title || '微').slice(0, 1)

  return (
    <TooltipProvider>
      <div className="min-h-dvh bg-background text-foreground">
        <a
          href="#messages"
          className="sr-only focus:not-sr-only focus:fixed focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-primary focus:px-3 focus:py-2 focus:text-primary-foreground"
        >
          跳到消息内容
        </a>
        <header className="sticky top-0 z-20 border-b bg-background/85 backdrop-blur">
          <div className="mx-auto flex max-w-4xl flex-wrap items-center gap-3 px-4 py-3">
            <Avatar
              size="lg"
              aria-hidden="true"
              className="size-11"
              style={{ width: 44, height: 44, backgroundColor: 'var(--primary)', color: 'var(--primary-foreground)' }}
            >
              {DATA.avatar && <AvatarImage src={DATA.avatar} alt="" />}
              <AvatarFallback className="bg-transparent text-base font-semibold text-current">
                {initial}
              </AvatarFallback>
            </Avatar>
            <div className="min-w-0 flex-1">
              <h1 className="truncate text-base font-semibold">{DATA.title}</h1>
              <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <Badge variant="secondary">{DATA.kind}</Badge>
                <span aria-live="polite" aria-atomic="true">
                  {query ? `${filtered.length} / ${DATA.messages.length} 条消息` : `共 ${DATA.messages.length} 条消息`}
                </span>
                <span className="hidden font-mono sm:inline">{DATA.id}</span>
                <span>导出于 {DATA.exportedAt}</span>
              </div>
            </div>
            <div className="ml-auto flex w-full items-center gap-2 sm:w-auto">
              <div role="search" className="min-w-0 flex-1 sm:w-64 sm:flex-none">
                <InputGroup className="h-12 max-w-none sm:h-9">
                  <InputGroupAddon>
                    <Search aria-hidden="true" />
                  </InputGroupAddon>
                  <InputGroupInput
                    ref={inputRef}
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="搜索消息…（/ 聚焦）"
                    aria-label="搜索消息"
                    className="h-full"
                  />
                </InputGroup>
              </div>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => setDark((value) => !value)}
                    aria-label="切换主题"
                    style={{ minWidth: 44, minHeight: 44 }}
                  >
                    {dark ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>切换主题</TooltipContent>
              </Tooltip>
            </div>
          </div>
        </header>

        <main id="messages" tabIndex={-1} className="mx-auto max-w-4xl scroll-mt-36 px-4 py-6 pb-24 outline-none sm:scroll-mt-20">
          <div className="mb-4 flex items-center justify-between text-xs text-muted-foreground">
            <span>{query ? `筛选出 ${filtered.length} 条` : '点击引用可跳转原消息'}</span>
            <span>/ 聚焦搜索 · Esc 清空</span>
          </div>

          {filtered.length === 0 ? (
            <Empty className="mt-10 border">
              <EmptyMedia variant="icon">
                <SearchX aria-hidden="true" />
              </EmptyMedia>
              <EmptyHeader>
                <EmptyTitle>没有匹配的消息</EmptyTitle>
                <EmptyDescription>试试其他关键词，或按 Esc 清空筛选。</EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setQuery('')
                    inputRef.current?.focus()
                  }}
                >
                  清空搜索
                </Button>
              </EmptyContent>
            </Empty>
          ) : (
            <div className="flex flex-col gap-3">
              {filtered.map((message, position) => {
                const previous = position > 0 ? filtered[position - 1] : null
                const showDay = !previous || previous.day !== message.day
                const showAvatar = DATA.isGroup && !message.mine && (!previous || previous.sender !== message.sender)
                return (
                  <div key={message.index} className="contents">
                    {showDay && (
                      <div className="my-3 flex items-center gap-3">
                        <Separator className="flex-1" />
                        <Badge variant="secondary">{message.day}</Badge>
                        <Separator className="flex-1" />
                      </div>
                    )}
                    <MessageRow
                      message={message}
                      flash={flash === message.index}
                      showAvatar={showAvatar}
                      onJump={jumpTo}
                    />
                  </div>
                )
              })}
            </div>
          )}
        </main>

        {showBottom && (
          <Button
            variant="outline"
            size="icon"
            onClick={() =>
              window.scrollTo({ top: document.body.scrollHeight, behavior: reducedMotion ? 'auto' : 'smooth' })
            }
            className="fixed right-5 bottom-5 z-20 rounded-full shadow-md"
            aria-label="回到最新"
          >
            <ArrowDown aria-hidden="true" />
          </Button>
        )}
      </div>
    </TooltipProvider>
  )
}

export default App
