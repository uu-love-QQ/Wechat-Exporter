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
}

export interface MissingMedia {
  kind: MediaKind
  name: string
}

export interface ChatMessage {
  index: number
  sender: string
  time: string
  day: string
  mine: boolean
  content: string
  quote: QuoteRef | null
  media: ChatMedia | null
  missing: MissingMedia | null
}

export interface ChatData {
  title: string
  kind: string
  id: string
  exportedAt: string
  messages: ChatMessage[]
}
