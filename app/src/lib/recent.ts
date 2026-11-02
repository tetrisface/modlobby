import {
	createEffect,
	createMemo,
	createSignal,
	onCleanup,
	type Accessor,
} from 'solid-js'
import type { OptionChangeView } from '../ipc/bindings/OptionChangeView'

/** How long a changed setting stays lit: long enough to look up and see what moved. */
export const RECENT_SECONDS = 30

const nowSeconds = () => Date.now() / 1000

const sameKeys = (a: ReadonlySet<string>, b: ReadonlySet<string>) =>
	a.size === b.size && [...a].every((key) => b.has(key))

/**
 * The options that changed a moment ago: what a preset or somebody's edit just
 * moved, which a re-sorted list does not otherwise say.
 *
 * Taken off by a timer rather than a CSS animation, so it reads the same with
 * reduced motion on. One timer, for the next one due; no change, no timer.
 */
export function createRecent(
	history: Accessor<readonly OptionChangeView[]>,
): Accessor<ReadonlySet<string>> {
	const [tick, setTick] = createSignal(0)

	const recent = createMemo<ReadonlySet<string>>(
		() => {
			tick()
			const since = nowSeconds() - RECENT_SECONDS
			return new Set(
				history()
					.filter((change) => change.at > since)
					.map((change) => change.key),
			)
		},
		new Set<string>(),
		{ equals: sameKeys },
	)

	createEffect(() => {
		tick()
		const now = nowSeconds()
		const due = history()
			.map((change) => change.at + RECENT_SECONDS)
			.filter((at) => at > now)
		if (due.length === 0) return
		const timer = setTimeout(
			() => setTick((turn) => turn + 1),
			(Math.min(...due) - now) * 1000,
		)
		onCleanup(() => clearTimeout(timer))
	})

	return recent
}
