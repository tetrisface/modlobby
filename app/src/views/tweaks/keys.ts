import { onCleanup } from 'solid-js'

/** The part of a Monaco editor this needs; a code editor is one. */
type Actionable = {
	focus(): void
	getAction(id: string): { run(): Promise<void> } | null
}

/**
 * With a tweak open, these reach it wherever focus is: Ctrl+F searches it
 * rather than the page around it, and Ctrl+P opens its command palette
 * rather than the webview's print dialog. Each names the editor action it runs.
 */
export const GRABBED: Readonly<Record<string, string>> = {
	f: 'actions.find',
	p: 'editor.action.quickCommand',
}

/** Routes the `GRABBED` keys to `editor` for as long as the calling component lives. */
export function grabKeys(editor: () => Actionable | undefined): void {
	const onKey = (event: KeyboardEvent) => {
		if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey)
			return
		const action = GRABBED[event.key.toLowerCase()]
		const target = editor()
		if (!action || !target) return
		event.preventDefault()
		event.stopPropagation()
		target.focus()
		void target.getAction(action)?.run()
	}
	window.addEventListener('keydown', onKey, true)
	onCleanup(() => window.removeEventListener('keydown', onKey, true))
}
