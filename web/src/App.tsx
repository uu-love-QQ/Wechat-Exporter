import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowDown,
  FileText,
  Moon,
  Search,
  SearchX,
  Sun,
  TriangleAlert,
} from 'lucide-react'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
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

function initialTheme(): boolean {
  if (typeof document === 'undefined') return false
  return document.documentElement.classList.contains('dark')
}

function MediaBlock({ media, mine }: { media: ChatMedia; mine: boolean }) {
  const href = media.path
  if (media.kind === 'image' || media.kind === 'emoji') {
    return (
      <a href={href} target="_blank" rel="noreferrer" className="mt-3 block w-fit">
        <img
          src={href}
          alt={media.name || KIND_LABEL[media.kind]}
          loading="lazy"
          className="max-h-[420px] max-w-full rounded-xl border object-contain"
        />
      </a>
    )
  }
  if (media.kind === 'video') {
    return <video src={href} controls preload="metadata" className="mt-3 w-full max-w-md rounded-xl" />
  }
  if (media.kind === 'voice') {
    return <audio src={href} controls preload="metadata" className="mt-3 w-full max-w-sm" />
  }
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className={cn(
        'mt-3 inline-flex items-center gap-2 rounded-lg border px-3 py-2 text-sm',
        mine
          ? 'border-primary-foreground/30 bg-background/15 text-primary-foreground'
          : 'border-border bg-muted/50 text-foreground hover:border-primary',
      )}
    >
      <FileText className="size-4 shrink-0" />
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
        {missing.name ? `：${missing.name}` : ''} 未在本地索引到，已留空
      </AlertDescription>
    </Alert>
  )
}

function MessageRow({
  message,
  flash,
  onJump,
}: {
  message: ChatMessage
  flash: boolean
  onJump: (index: number) => void
}) {
  const mine = message.mine
  return (
    <div id={`msg-${message.index}`} className={cn('flex scroll-mt-24', mine ? 'justify-end' : 'justify-start')}>
      <div
        className={cn(
          'max-w-[78%] rounded-2xl border px-4 py-3 shadow-sm transition-shadow',
          mine
            ? 'border-primary/40 bg-primary text-primary-foreground'
            : 'border-border bg-card text-card-foreground',
          flash && 'ring-2 ring-ring ring-offset-2 ring-offset-background',
        )}
      >
        <div
          className={cn(
            'mb-1 flex flex-wrap items-baseline gap-x-2 gap-y-0.5 text-xs',
            mine ? 'justify-end text-primary-foreground/70' : 'text-muted-foreground',
          )}
        >
          <span className={cn('font-medium', mine ? 'text-primary-foreground' : 'text-foreground')}>
            {message.sender}
          </span>
          <time>{message.time}</time>
        </div>

        {message.quote && (
          <div
            className={cn(
              'mb-2 rounded-lg border-l-2 p-2 text-sm',
              mine
                ? 'border-primary-foreground/40 bg-background/15'
                : message.quote.target === null
                  ? 'border-destructive/50 bg-muted/60'
                  : 'border-primary bg-muted/60',
            )}
          >
            <div className="flex items-center gap-2 text-xs">
              <span className={cn('font-medium', mine ? 'text-primary-foreground' : 'text-foreground')}>
                {message.quote.sender || '未知'}
              </span>
              {message.quote.target !== null ? (
                <button
                  type="button"
                  onClick={() => onJump(message.quote!.target!)}
                  className={cn('ml-auto underline-offset-2 hover:underline', mine ? 'text-primary-foreground' : 'text-primary')}
                >
                  查看原消息
                </button>
              ) : (
                <span className={cn('ml-auto text-xs', mine ? 'text-primary-foreground/70' : 'text-muted-foreground')}>
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
                  mine ? 'text-primary-foreground/80' : 'text-muted-foreground',
                )}
              >
                {message.quote.text}
              </button>
            ) : (
              <div
                className={cn(
                  'mt-1 whitespace-pre-wrap',
                  mine ? 'text-primary-foreground/90' : 'text-foreground/90',
                )}
              >
                {message.quote.text}
              </div>
            )}
          </div>
        )}

        {message.content && (
          <div className="whitespace-pre-wrap break-words text-sm leading-relaxed">{message.content}</div>
        )}

        {message.media && <MediaBlock media={message.media} mine={mine} />}
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
      target.scrollIntoView({ behavior: 'smooth', block: 'start' })
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
        <header className="sticky top-0 z-20 border-b bg-background/85 backdrop-blur">
          <div className="mx-auto flex max-w-4xl flex-wrap items-center gap-3 px-4 py-3">
            <Avatar size="lg">
              <AvatarFallback>{initial}</AvatarFallback>
            </Avatar>
            <div className="min-w-0 flex-1">
              <h1 className="truncate text-base font-semibold">{DATA.title}</h1>
              <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <Badge variant="secondary">{DATA.kind}</Badge>
                <span>{query ? `${filtered.length} / ${DATA.messages.length} 条消息` : `共 ${DATA.messages.length} 条消息`}</span>
                <span className="hidden font-mono sm:inline">{DATA.id}</span>
                <span>导出于 {DATA.exportedAt}</span>
              </div>
            </div>
            <div className="ml-auto flex w-full items-center gap-2 sm:w-auto">
              <InputGroup className="max-w-none flex-1 sm:w-64 sm:flex-none">
                <InputGroupAddon>
                  <Search />
                </InputGroupAddon>
                <InputGroupInput
                  ref={inputRef}
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="搜索消息…（/ 聚焦）"
                  aria-label="搜索消息"
                />
              </InputGroup>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => setDark((value) => !value)}
                    aria-label="切换主题"
                  >
                    {dark ? <Sun /> : <Moon />}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>切换主题</TooltipContent>
              </Tooltip>
            </div>
          </div>
        </header>

        <main className="mx-auto max-w-4xl px-4 py-6 pb-24">
          <div className="mb-4 flex items-center justify-between text-xs text-muted-foreground">
            <span>{query ? `筛选出 ${filtered.length} 条` : '点击引用可跳转原消息'}</span>
            <span>/ 聚焦搜索 · Esc 清空</span>
          </div>

          {filtered.length === 0 ? (
            <Empty className="mt-10 border">
              <EmptyMedia variant="icon">
                <SearchX />
              </EmptyMedia>
              <EmptyHeader>
                <EmptyTitle>没有匹配的消息</EmptyTitle>
                <EmptyDescription>试试其他关键词，或按 Esc 清空筛选。</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <div className="flex flex-col gap-3">
              {filtered.map((message, position) => {
                const showDay = position === 0 || filtered[position - 1].day !== message.day
                return (
                  <div key={message.index} className="contents">
                    {showDay && (
                      <div className="my-3 flex items-center gap-3">
                        <Separator className="flex-1" />
                        <Badge variant="secondary">{message.day}</Badge>
                        <Separator className="flex-1" />
                      </div>
                    )}
                    <MessageRow message={message} flash={flash === message.index} onJump={jumpTo} />
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
            onClick={() => window.scrollTo({ top: document.body.scrollHeight, behavior: 'smooth' })}
            className="fixed right-5 bottom-5 z-20 rounded-full shadow-md"
            aria-label="回到最新"
          >
            <ArrowDown />
          </Button>
        )}
      </div>
    </TooltipProvider>
  )
}

export default App
