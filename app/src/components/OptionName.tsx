import { type JSX, Show, createSignal } from 'solid-js'
import { Portal } from 'solid-js/web'
import type { ModOption } from '../ipc/bindings/ModOption'
import { hint, label, tipFacts } from '../lib/setup'
import { Marked } from './Marked'

/**
 * An option's name, and on hover its Lua key set apart from what BAR says
 * of it, then what its table says: its range, or its choices, and its default.
 * Not a `title`: a native tooltip cannot style the key.
 *
 * Only the name answers, and the stylesheet's little more around it: the rest of
 * the row is for reading across. `children` go inside the name, where one with a
 * tip of its own keeps it, and `after` beside it.
 */
export function OptionName(props: {
	option: ModOption
	children?: JSX.Element
	after?: JSX.Element
}) {
	const [at, setAt] = createSignal<DOMRect>()
	let name: HTMLSpanElement | undefined

	function over(event: PointerEvent) {
		const tipped = (event.target as Element).closest('[title]')
		setAt(
			tipped !== null && name?.contains(tipped)
				? undefined
				: name?.getBoundingClientRect(),
		)
	}

	return (
		<span class='opt-name'>
			<span
				class='k'
				ref={name}
				aria-description={hint(props.option)}
				onPointerOver={over}
				onPointerLeave={() => setAt()}
			>
				<Marked text={label(props.option)} />
				{props.children}
			</span>
			{props.after}
			<Show when={at()}>
				{(rect) => (
					<Portal>
						<span class='key-tip' role='tooltip' style={place(rect())}>
							<code>{props.option.key}:</code>
							{props.option.desc && ` ${props.option.desc}`}
							<Show when={tipFacts(props.option)}>
								{(told) => <span class='key-tip-facts'>{told()}</span>}
							</Show>
						</span>
					</Portal>
				)}
			</Show>
		</span>
	)
}

/**
 * Fixed, and drawn at the document's own level: a container query's containment
 * makes some webviews measure `fixed` from the container, and the row's ellipsis
 * and the pane's scroller would clip it. Above the name in the window's lower half.
 */
function place(name: DOMRect): JSX.CSSProperties {
	const left = `${name.left}px`
	const width = `min(32rem, ${innerWidth - name.left - 12}px)`
	return name.top > innerHeight / 2
		? { left, bottom: `${innerHeight - name.top + 4}px`, 'max-width': width }
		: { left, top: `${name.bottom + 4}px`, 'max-width': width }
}
