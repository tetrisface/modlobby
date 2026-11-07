import { Show, createMemo } from 'solid-js'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import { type Entry, keep, segment } from '../lib/hostTable'
import { Linkify } from './Linkify'

/** A log's lines as it draws them: host tables gathered, each kept while unchanged. */
export function createEntries(lines: () => readonly ChatLine[]) {
	return createMemo<Entry[]>((previous) => keep(previous, segment(lines())), [])
}

/**
 * What a line says, or the table it opens. A table keeps SPADS's padding in
 * a fixed-width face, which is what lines its columns up, and scrolls rather
 * than wraps when the log is narrower.
 */
export function ChatText(props: { line: ChatLine; rows?: string[] }) {
	return (
		<Show
			when={props.rows}
			fallback={
				<span class='text'>
					<Linkify text={props.line.text} />
				</span>
			}
		>
			{(rows) => <pre class='text host-table'>{rows().join('\n')}</pre>}
		</Show>
	)
}
