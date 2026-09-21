import { createEffect, onCleanup, onMount } from 'solid-js'
import {
	createEditor,
	idOfModel,
	monaco,
	setProblems,
	switchModel,
} from '../../editor/monaco'
import type { Problem } from '../../ipc/bindings/Problem'
import type { Symbol } from '../../ipc/bindings/Symbol'
import type { Assist, Warning } from '../../lib/assist'
import { KINDS, type Doc, type DocId } from '../../lib/tweakspace'
import { grabKeys } from './keys'
import { registerAssist } from './providers'

/** A place to go, stamped so that going to the same line twice still goes. */
export type Goto = { line: number; column: number; at: number }

/**
 * The one place the workspace touches Monaco for editing.
 *
 * Text flows one way at a time: what is typed goes to the store on every
 * change, and the store's text comes back only through `switchModel`, which
 * seeds the model when the store has moved on its own -- a format, a reset,
 * the room's value arriving. Comparing before seeding is what keeps the two
 * from chasing each other.
 */
export function EditorHost(props: {
	doc: Doc
	problems: Problem[]
	warnings: Warning[]
	assist: Assist
	goto: Goto | null
	/** Only where there is width to spare for it: over the whole window. */
	minimap: boolean
	/** What the Rust check found at the top level; Ctrl+Shift+O lists it. */
	outline: Symbol[]
	/** Format Document (Shift+Alt+F), as the Format button formats. */
	format: (text: string) => Promise<string>
	onEdit: (id: DocId, text: string) => void
	/** Ctrl+S: keep it as a draft. */
	onSave: () => void
	/** Ctrl+Enter: what the send bar's primary button does. */
	onSend: () => void
}) {
	let host: HTMLDivElement | undefined
	let editor: monaco.editor.IStandaloneCodeEditor | undefined
	const language = () => KINDS[props.doc.kind].language
	grabKeys(() => editor)

	onMount(() => {
		if (!host) return
		editor = createEditor(host, {
			minimap: { enabled: props.minimap },
			wordWrap: 'on',
			folding: true,
			bracketPairColorization: { enabled: true },
			insertSpaces: false,
			// StyLua indents every scope, so indentation is the outline; the
			// Rust check's symbols are single lines and would pin nothing.
			stickyScroll: { enabled: true, defaultModel: 'indentationModel' },
			mouseWheelZoom: true,
		})
		editor.onDidChangeModelContent(() => {
			const model = editor?.getModel()
			if (!model) return
			props.onEdit(idOfModel(model), model.getValue())
		})
		editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () =>
			props.onSave(),
		)
		editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () =>
			props.onSend(),
		)
		// The editor's own font size; the app's scale is the app's. Monaco
		// ships the actions without keys.
		const zoom: Array<[number, string]> = [
			[monaco.KeyCode.Equal, 'editor.action.fontZoomIn'],
			[monaco.KeyCode.Minus, 'editor.action.fontZoomOut'],
			[monaco.KeyCode.Digit0, 'editor.action.fontZoomReset'],
		]
		for (const [key, action] of zoom)
			editor.addCommand(monaco.KeyMod.CtrlCmd | key, () =>
				editor?.trigger('keyboard', action, null),
			)
		switchModel(editor, props.doc.id, props.doc.buffer, language())
		registerAssist(() => ({
			assist: props.assist,
			kind: props.doc.kind,
			outline: props.outline,
			format: props.format,
		}))
	})

	createEffect(() => {
		const { id, buffer } = props.doc
		if (editor) switchModel(editor, id, buffer, language())
	})

	// The problems are the active document's; on a switch they are cleared
	// until the check for the new one arrives.
	createEffect(() => {
		const model = editor?.getModel()
		if (model) setProblems(model, props.problems, props.warnings)
	})

	createEffect(() => {
		const target = props.goto
		if (!target || !editor) return
		editor.revealLineInCenter(target.line)
		editor.setPosition({ lineNumber: target.line, column: target.column })
		editor.focus()
	})

	onCleanup(() => editor?.dispose())

	return <div class='tweak-editor' ref={host} />
}
