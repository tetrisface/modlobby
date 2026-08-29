import { createStore } from 'solid-js/store'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import type { NoticeLevel } from '../ipc/bindings/NoticeLevel'

export type Notice = { seq: number; level: NoticeLevel; text: string }

export type ChatState = {
  lines: ChatLine[]
  notices: Notice[]
  maxLines: number
}

export const [chat, setChat] = createStore<ChatState>({
  lines: [],
  notices: [],
  maxLines: 500,
})

let noticeSeq = 0

export function pushLine(line: ChatLine): void {
  setChat('lines', (lines) => {
    const next = [...lines, line]
    return next.length > chat.maxLines
      ? next.slice(next.length - chat.maxLines)
      : next
  })
}

export function pushNotice(level: NoticeLevel, text: string): void {
  noticeSeq += 1
  setChat('notices', (notices) => [
    ...notices.slice(-19),
    { seq: noticeSeq, level, text },
  ])
}

export function clearChat(): void {
  setChat({ lines: [], notices: [] })
}
