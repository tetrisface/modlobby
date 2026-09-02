import { monaco } from '../../editor/monaco'
import type { Kind } from '../../ipc/bindings/Kind'
import {
  describeTag,
  pathAt,
  suggestions,
  tagAt,
  type Assist,
} from '../../lib/assist'

let registered = false

/**
 * Completion and hover for a `tweakunits` table, from what the game and the
 * engine know. Registered once for the Lua language; the accessor is read
 * on every request, so a new room's units are offered without re-registering.
 *
 * Only models from the workspace (`tweak:` scheme) and only for the units
 * kind: a `tweakdefs` script has braces that mean other things.
 */
export function registerAssist(
  current: () => { assist: Assist; kind: Kind },
): void {
  if (registered) return
  registered = true

  const applies = (model: monaco.editor.ITextModel) =>
    model.uri.scheme === 'tweak' && current().kind === 'units'

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
