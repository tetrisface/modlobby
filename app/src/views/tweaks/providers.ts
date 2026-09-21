import { monaco } from '../../editor/monaco'
import type { Kind } from '../../ipc/bindings/Kind'
import type { Symbol } from '../../ipc/bindings/Symbol'
import { describeError } from '../../ipc/client'
import {
	describeTag,
	pathAt,
	suggestions,
	tagAt,
	type Assist,
} from '../../lib/assist'
import { pushNotice } from '../../store/chat'

/** What the providers need of the open document, asked on every request. */
export type AssistContext = {
	assist: Assist
	kind: Kind
	/** What the Rust check found at the top level, for the outline picker. */
	outline: Symbol[]
	/** StyLua, or the JSON formatter for the override. */
	format: (text: string) => Promise<string>
}

let registered = false
/** The editor mounted last; it is the one open, since only one ever is. */
let current: () => AssistContext

/**
 * Completion and hover for a `tweakunits` table, from what the game and the
 * engine know; Format Document through our formatter; and the outline the
 * Rust check found, for Ctrl+Shift+O. Registered once per language, reading
 * whichever editor was mounted last -- so a new room's units are offered
 * without re-registering, and a closed editor is never asked.
 *
 * Only models from the workspace (`tweak:` scheme). Completion and hover only
 * for the units kind: a `tweakdefs` script has braces that mean other things.
 */
export function registerAssist(context: () => AssistContext): void {
	current = context
	if (registered) return
	registered = true

	const ours = (model: monaco.editor.ITextModel) => model.uri.scheme === 'tweak'
	const applies = (model: monaco.editor.ITextModel) =>
		ours(model) && current().kind === 'units'

	for (const language of ['lua', 'json'])
		monaco.languages.registerDocumentFormattingEditProvider(language, {
			async provideDocumentFormattingEdits(model) {
				if (!ours(model)) return []
				try {
					const text = await current().format(model.getValue())
					return [{ range: model.getFullModelRange(), text }]
				} catch (error) {
					pushNotice('warning', `format: ${describeError(error)}`)
					return []
				}
			},
		})

	monaco.languages.registerDocumentSymbolProvider('lua', {
		provideDocumentSymbols(model) {
			if (!ours(model)) return []
			const { outline, kind } = current()
			return outline
				.filter((symbol) => symbol.line <= model.getLineCount())
				.map((symbol) => {
					const range = new monaco.Range(
						symbol.line,
						1,
						symbol.line,
						model.getLineMaxColumn(symbol.line),
					)
					return {
						name: symbol.name,
						detail: '',
						kind:
							kind === 'units'
								? monaco.languages.SymbolKind.Field
								: monaco.languages.SymbolKind.Variable,
						tags: [],
						range,
						selectionRange: range,
					}
				})
		},
	})

	monaco.languages.registerCompletionItemProvider('lua', {
		provideCompletionItems(model, position) {
			if (!applies(model)) return { suggestions: [] }
			const path = pathAt(model.getValue(), model.getOffsetAt(position))
			const word = model.getWordUntilPosition(position)
			const range = new monaco.Range(
				position.lineNumber,
				word.startColumn,
				position.lineNumber,
				word.endColumn,
			)
			return {
				suggestions: suggestions(path, current().assist).map((entry) => ({
					label: entry.name,
					kind: monaco.languages.CompletionItemKind.Field,
					detail: entry.detail,
					documentation: entry.doc,
					insertText: entry.name,
					range,
				})),
			}
		},
	})

	monaco.languages.registerHoverProvider('lua', {
		provideHover(model, position) {
			if (!applies(model)) return null
			const word = model.getWordAtPosition(position)
			if (!word) return null
			const { assist } = current()
			const path = pathAt(
				model.getValue(),
				model.getOffsetAt({
					lineNumber: position.lineNumber,
					column: word.startColumn,
				}),
			)
			const lines = describe(path, word.word, assist)
			if (lines.length === 0) return null
			return {
				range: new monaco.Range(
					position.lineNumber,
					word.startColumn,
					position.lineNumber,
					word.endColumn,
				),
				contents: lines.map((value) => ({ value })),
			}
		},
	})
}

/** What the hover says about a word, as markdown lines; nothing when nothing is known. */
export function describe(
	path: string[],
	word: string,
	assist: Assist,
): string[] {
	if (path.length === 1 && assist.units.length > 0) {
		return assist.units.includes(word.toLowerCase())
			? [`**${word}** · a unit in this game`]
			: [`**${word}** · not a unit in this game; the tweak skips it`]
	}
	const tag = tagAt(path, word, assist)
	if (!tag) return []
	const lines = [`**${tag.name}** · ${describeTag(tag)}`]
	if (tag.min !== null || tag.max !== null)
		lines.push(`from ${tag.min ?? '…'} to ${tag.max ?? '…'}`)
	if (tag.description) lines.push(tag.description)
	return lines
}
