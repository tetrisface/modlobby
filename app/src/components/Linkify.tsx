import { For } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'

/**
 * Chat text with its links made clickable.
 *
 * Hosts and players post replay links, map pages and documents constantly, and
 * copying them out of a line by hand is the kind of small friction that makes a
 * client feel unfinished. Links open in the system browser rather than in the
 * webview — this window is the lobby, not a browser, and a page loaded inside
 * it would sit on the app's own origin.
 *
 * The text is rendered as text nodes, never as markup: every character here was
 * written by somebody else.
 */

/** Trailing punctuation usually belongs to the sentence, not the address. */
const TRAILING = /[.,;:!?)\]}'"]+$/

const URLS = /\bhttps?:\/\/[^\s<>"']+/gi

type Piece = { text: string; href?: string }

export function split(text: string): Piece[] {
  const pieces: Piece[] = []
  let at = 0

  for (const match of text.matchAll(URLS)) {
    const start = match.index
    if (start > at) pieces.push({ text: text.slice(at, start) })

    const raw = match[0]
    const trimmed = raw.replace(TRAILING, '')
    pieces.push({ text: trimmed, href: trimmed })
    if (trimmed.length < raw.length) {
      pieces.push({ text: raw.slice(trimmed.length) })
    }
    at = start + raw.length
  }

  if (at < text.length) pieces.push({ text: text.slice(at) })
  return pieces
}

export function Linkify(props: { text: string }) {
  async function open(href: string) {
    try {
      await api.openUrl(href)
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  return (
    <For each={split(props.text)}>
      {(piece) =>
        piece.href ? (
          <a
            class='chat-link'
            href={piece.href}
            title={piece.href}
            onClick={(event) => {
              event.preventDefault()
              void open(piece.href!)
            }}
          >
            {piece.text}
          </a>
        ) : (
          piece.text
        )
      }
    </For>
  )
}
