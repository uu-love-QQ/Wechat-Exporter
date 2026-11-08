export type MediaKind = 'image' | 'video' | 'voice' | 'emoji' | 'file'

export interface QuoteRef {
  sender: string
  text: string
  target: number | null
}

export interface ChatMedia {
  path: string
  kind: MediaKind
  name: string
  width?: number | null
  height?: number | null
  durationMs?: number | null
}

export interface MissingMedia {
  kind: MediaKind
  name: string
  durationMs?: number | null
}

export interface ChatRecordItem {
  type: number | null
  text: string
  sender: string
  time: string
}

export interface ChatRecord {
  title: string
  desc: string
  items: ChatRecordItem[]
}

export interface ChatMessage {
  index: number
  sender: string
  time: string
  day: string
  mine: boolean
  content: string
  quote: QuoteRef | null
  record: ChatRecord | null
  media: ChatMedia | null
  missing: MissingMedia | null
}

export interface ChatData {
  title: string
  kind: string
  isGroup: boolean
  id: string
  exportedAt: string
  avatar?: string | null
  messages: ChatMessage[]
}
