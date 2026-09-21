import { Show } from 'solid-js'

/**
 * The window corner's glyphs, for anything else that fills the window or
 * closes: one look for the same two acts, wherever they are.
 */
export function FullscreenGlyph(props: { full: boolean }) {
	return (
		<svg viewBox='0 0 12 12' aria-hidden='true'>
			<Show
				when={props.full}
				fallback={
					// Corners pointing out: take the whole screen.
					<path d='M1 4V1h3M8 1h3v3M11 8v3H8M4 11H1V8' />
				}
			>
				{/* Corners pointing in: back to where it was. */}
				<path d='M4 1v3H1M11 4H8V1M8 11V8h3M1 8h3v3' />
			</Show>
		</svg>
	)
}

/** VS Code's: one bar, low in the box, where the window goes. */
export function MinimizeGlyph() {
	return (
		<svg viewBox='0 0 12 12' aria-hidden='true'>
			<path d='M2 6.5h8' />
		</svg>
	)
}

export function CloseGlyph() {
	return (
		<svg viewBox='0 0 12 12' aria-hidden='true'>
			<path d='M2 2l8 8M10 2l-8 8' />
		</svg>
	)
}
