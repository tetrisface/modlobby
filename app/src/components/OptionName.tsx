import { type JSX, Show, createSignal } from 'solid-js'
import type { ModOption } from '../ipc/bindings/ModOption'
import { hint, label } from '../lib/setup'

/**
 * An option's name, and on hover its Lua key set apart from what BAR says
 * of it. Not a `title`: a native tooltip cannot style the key.
 */
export function OptionName(props: {
	option: ModOption
	children?: JSX.Element
}) {
	const [at, setAt] = createSignal<DOMRect>()
	return (
		<span
			class='k'
			aria-description={hint(props.option)}
			onPointerEnter={(event) =>
				setAt(event.currentTarget.getBoundingClientRect())
			}
			onPointerLeave={() => setAt()}
		>
			{label(props.option)}
			{props.children}
			<Show when={at()}>
				{(name) => (
					<span class='key-tip' role='tooltip' style={place(name())}>
						<code>{props.option.key}:</code>
						{props.option.desc && ` ${props.option.desc}`}
					</span>
				)}
			</Show>
		</span>
	)
}

/** Fixed, so the row's ellipsis and the pane's scroller cannot clip it; above the name in the window's lower half. */
function place(name: DOMRect): JSX.CSSProperties {
	const left = `${name.left}px`
	const width = `min(32rem, ${innerWidth - name.left - 12}px)`
	return name.top > innerHeight / 2
		? { left, bottom: `${innerHeight - name.top + 4}px`, 'max-width': width }
		: { left, top: `${name.bottom + 4}px`, 'max-width': width }
}
