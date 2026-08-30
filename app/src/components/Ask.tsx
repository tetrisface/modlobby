import { Show, createSignal, onMount } from 'solid-js'

/**
 * Asking for one line of text without stopping the lobby.
 *
 * Never `window.prompt`: it blocks the whole page until it is answered — chat,
 * the battle list, every timer — and WebKitGTK, the webview everywhere that is
 * not Windows, refuses it outright and hands back `null`.
 */
export function Ask(props: {
  title: string
  hint?: string
  initial?: string
  confirm?: string
  onAnswer: (text: string) => void
  onCancel: () => void
}) {
  const [text, setText] = createSignal(props.initial ?? '')
  let field: HTMLInputElement | undefined

  onMount(() => {
    field?.focus()
    field?.select()
  })

  return (
    <div class='sheet' onMouseDown={props.onCancel}>
      <form
        class='sheet-card'
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault()
          const answer = text().trim()
          if (answer) props.onAnswer(answer)
        }}
      >
        <h2>{props.title}</h2>
        <Show when={props.hint}>{(hint) => <p class='muted'>{hint()}</p>}</Show>
        <input
          ref={field}
          value={text()}
          onInput={(event) => setText(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') props.onCancel()
          }}
        />
        <div class='sheet-actions'>
          <button type='button' onClick={props.onCancel}>
            Cancel
          </button>
          <button class='primary' type='submit' disabled={!text().trim()}>
            {props.confirm ?? 'OK'}
          </button>
        </div>
      </form>
    </div>
  )
}
