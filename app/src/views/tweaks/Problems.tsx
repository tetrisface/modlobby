import { For, Show } from 'solid-js'
import type { Problem } from '../../ipc/bindings/Problem'
import type { Warning } from '../../lib/assist'

/**
 * Where the Lua stops making sense, one line each, and the notes that came
 * with the decoded slot. Open whenever there is something in it: the gauge
 * goes quiet on a syntax error, and this is what says why.
 */
export function Problems(props: {
  problems: Problem[]
  /** Lines the game would skip: a unit it does not have. */
  warnings: Warning[]
  notes: string[]
  onGoto: (line: number, column: number) => void
}) {
  const count = () =>
    props.problems.length + props.warnings.length + props.notes.length
  return (
    <Show when={count() > 0}>
      <details class='tweak-extra problems' open>
        <summary>Problems · {count()}</summary>
        <For each={props.notes}>{(note) => <p class='error'>{note}</p>}</For>
        <ul class='problem-list'>
          <For each={props.problems}>
            {(problem) => (
              <li>
                <button
                  class='link'
                  onClick={() => props.onGoto(problem.line, problem.column)}
                >
                  {problem.line}:{problem.column}
                </button>{' '}
                {problem.message}
              </li>
            )}
          </For>
          <For each={props.warnings}>
            {(warning) => (
              <li class='warn'>
                <button
                  class='link'
                  onClick={() => props.onGoto(warning.line, 1)}
                >
                  {warning.line}
                </button>{' '}
                {warning.message}
              </li>
            )}
          </For>
        </ul>
      </details>
    </Show>
  )
}
