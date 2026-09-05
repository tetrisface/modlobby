import { createSignal } from 'solid-js'
import { complete, completions, recall, remember, wordAt } from '../lib/compose'

/**
 * The box you type a line into.
 *
 * Shared by the channels and the battle room because the two behaviours worth
 * having — walking back through what you sent, and finishing a name with Tab —
 * are worth having in both, and are the sort of thing that quietly diverges
 * when written twice.
 *
 * The history is per-box and per-session: a lobby is not a shell, and nobody
 * expects yesterday's `!bSet` back.
 *
 * It is a textarea rather than an input so a pasted block keeps its line
 * breaks: the runtime sends one message per line, and an input would have
 * glued a whole preset into one line the server refuses. Enter sends;
 * Shift+Enter is a line break.
 */
export function Composer(props: {
  placeholder: string
  /** Whose names Tab may finish. Read on each press, never cached. */
  names: () => string[]
  onSend: (text: string) => void
}) {
  const [text, setText] = createSignal('')
  const [history, setHistory] = createSignal<string[]>([])
  const [at, setAt] = createSignal(-1)
  let input: HTMLTextAreaElement | undefined
  /** What was half-typed when the walk back started. */
  let draft = ''
  /**
   * A run of Tab presses on one word. Kept whole so each press can rebuild
   * from the original word rather than from the name the last press inserted.
   */
  let cycle: {
    base: string
    baseCaret: number
    word: string
    index: number
    produced: string
    producedCaret: number
  } | null = null

  /** Solid owns the value, so the caret has to be placed after it lands. */
  function put(next: string, caret: number) {
    setText(next)
    queueMicrotask(() => input?.setSelectionRange(caret, caret))
  }

  function onTab(event: KeyboardEvent) {
    event.preventDefault()
    const element = input
    if (!element) return
    const caret = element.selectionStart ?? element.value.length

    const again =
      cycle !== null &&
      element.value === cycle.produced &&
      caret === cycle.producedCaret
    const base = again ? cycle!.base : element.value
    const baseCaret = again ? cycle!.baseCaret : caret
    const word = again ? cycle!.word : wordAt(base, baseCaret).word
    if (!word) return

    const options = completions(word, props.names())
    if (options.length === 0) return
    const index = again ? cycle!.index + 1 : 0
    const filled = complete(base, baseCaret, options[index % options.length]!)

    cycle = {
      base,
      baseCaret,
      word,
      index,
      produced: filled.text,
      producedCaret: filled.caret,
    }
    put(filled.text, filled.caret)
  }

  function walk(step: -1 | 1, event: KeyboardEvent) {
    if (history().length === 0) return
    event.preventDefault()
    if (at() === -1) draft = text()
    const found = recall(history(), at(), step, draft)
    setAt(found.at)
    put(found.text, found.text.length)
  }

  function onEnter(event: KeyboardEvent) {
    if (event.shiftKey) return
    submit(event)
  }

  function submit(event: Event) {
    event.preventDefault()
    const line = text()
    if (!line.trim()) return
    setHistory(remember(history(), line))
    setAt(-1)
    draft = ''
    cycle = null
    setText('')
    props.onSend(line)
  }

  return (
    <form class='chat-input' onSubmit={submit}>
      <textarea
        ref={input}
        rows={1}
        value={text()}
        placeholder={props.placeholder}
        onInput={(event) => {
          setText(event.currentTarget.value)
          // Typing ends both the walk back and the run of completions.
          setAt(-1)
          cycle = null
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter') return onEnter(event)
          if (event.key === 'Tab' && !event.shiftKey) return onTab(event)
          if (event.key === 'ArrowUp') return walk(1, event)
          if (event.key === 'ArrowDown') return walk(-1, event)
        }}
      />
      <button type='submit'>Send</button>
    </form>
  )
}
