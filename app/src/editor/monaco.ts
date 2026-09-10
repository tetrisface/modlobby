// Only the editor core, Lua and JSON are pulled in; Monaco 0.56 keeps each
// language behind `languages/definitions/<id>/register`, so the rest never
// ships. Its exports map rewrites subpaths, so these are
// `monaco-editor/<path under esm/vs>`.
import * as monaco from 'monaco-editor/editor/editor.api'
import 'monaco-editor/languages/definitions/lua/register'
import { jsonDefaults } from 'monaco-editor/languages/features/json/register'
import EditorWorker from 'monaco-editor/editor/editor.worker?worker'
import type { Problem } from '../ipc/bindings/Problem'
import type { Warning } from '../lib/assist'
import type { DocId } from '../lib/tweakspace'

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment
  }
}

// Lua has no language service, so the plain editor worker is the only one needed.
self.MonacoEnvironment = { getWorker: () => new EditorWorker() }

// JSON has one, but only its colouring is wanted: the squiggles come from
// the Rust check, as they do for Lua, and everything else the service does
// would want a worker of its own.
jsonDefaults.setModeConfiguration({ tokens: true })

export { monaco }

/** What a document is highlighted as; see `KINDS` in `lib/tweakspace`. */
export type Language = 'lua' | 'json'

const OPTIONS: monaco.editor.IStandaloneEditorConstructionOptions = {
  language: 'lua',
  theme: 'vs-dark',
  automaticLayout: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 13,
  tabSize: 2,
  renderWhitespace: 'selection',
}

/** StyLua writes tabs, and a tab is two columns wide everywhere we draw one. */
const MODEL_OPTIONS: monaco.editor.ITextModelUpdateOptions = {
  tabSize: 2,
  insertSpaces: false,
}

export function createEditor(
  host: HTMLElement,
  overrides: monaco.editor.IStandaloneEditorConstructionOptions = {},
): monaco.editor.IStandaloneCodeEditor {
  return monaco.editor.create(host, { ...OPTIONS, ...overrides })
}

export function createDiffEditor(
  host: HTMLElement,
  original: string,
  modified: string,
  overrides: monaco.editor.IStandaloneDiffEditorConstructionOptions = {},
  language: Language = 'lua',
): monaco.editor.IStandaloneDiffEditor {
  const editor = monaco.editor.createDiffEditor(host, {
    ...OPTIONS,
    readOnly: true,
    renderSideBySide: true,
    ...overrides,
  })
  editor.setModel({
    original: monaco.editor.createModel(original, language),
    modified: monaco.editor.createModel(modified, language),
  })
  return editor
}

export function disposeDiff(editor: monaco.editor.IStandaloneDiffEditor): void {
  const model = editor.getModel()
  editor.dispose()
  model?.original.dispose()
  model?.modified.dispose()
}

/*
 * One model per document, kept for as long as the document is.
 *
 * A model is where Monaco keeps the undo history, so switching the editor
 * between models rather than pouring text into one is what lets Ctrl+Z still
 * work after you have looked at another slot and come back -- and what lets
 * the editor be torn down and put back without losing anything. The view
 * state (cursor, scroll, folds) is kept beside it for the same reason.
 */
const models = new Map<string, monaco.editor.ITextModel>()
const ids = new WeakMap<monaco.editor.ITextModel, string>()
const views = new Map<string, monaco.editor.ICodeEditorViewState | null>()

/** The document's model, made on first sight and brought up to `text`. */
export function modelFor(
  id: string,
  text: string,
  language: Language,
): monaco.editor.ITextModel {
  const found = models.get(id)
  if (found) {
    seedModel(found, text)
    return found
  }
  const model = monaco.editor.createModel(
    text,
    language,
    monaco.Uri.from({ scheme: 'tweak', path: `/${id}.${language}` }),
  )
  model.updateOptions(MODEL_OPTIONS)
  models.set(id, model)
  ids.set(model, id)
  return model
}

/** Shows a document, remembering where the reader was in the last one. */
export function switchModel(
  editor: monaco.editor.IStandaloneCodeEditor,
  id: string,
  text: string,
  language: Language,
): void {
  const current = editor.getModel()
  const leaving = current ? ids.get(current) : undefined
  if (leaving !== undefined) views.set(leaving, editor.saveViewState())
  if (leaving === id) {
    seedModel(current!, text)
    return
  }
  editor.setModel(modelFor(id, text, language))
  const view = views.get(id)
  if (view) editor.restoreViewState(view)
}

/** Which document a model is, for an edit that arrives through the model. */
export function idOfModel(model: monaco.editor.ITextModel): DocId {
  return (ids.get(model) ??
    model.uri.path.slice(1).replace(/\.\w+$/, '')) as DocId
}

/**
 * The squiggles: red where the text stops making sense, per the Rust check;
 * yellow along a line the game would skip.
 */
export function setProblems(
  model: monaco.editor.ITextModel,
  problems: Problem[],
  warnings: Warning[] = [],
): void {
  monaco.editor.setModelMarkers(model, 'check', [
    ...problems.map((problem) => ({
      severity: monaco.MarkerSeverity.Error,
      message: problem.message,
      startLineNumber: problem.line,
      startColumn: problem.column,
      endLineNumber: problem.endLine,
      endColumn: problem.endColumn,
    })),
    ...warnings.map((warning) => ({
      severity: monaco.MarkerSeverity.Warning,
      message: warning.message,
      startLineNumber: warning.line,
      startColumn: 1,
      endLineNumber: warning.line,
      endColumn: model.getLineMaxColumn(warning.line),
    })),
  ])
}

export function dropModel(id: string): void {
  models.get(id)?.dispose()
  models.delete(id)
  views.delete(id)
}

/**
 * Replaces a model's text through the edit stack rather than `setValue`, so
 * a reload or a format that clobbers work is still one Ctrl+Z away.
 */
export function seedModel(model: monaco.editor.ITextModel, text: string): void {
  if (model.getValue() === text) return
  model.pushStackElement()
  model.pushEditOperations(
    [],
    [{ range: model.getFullModelRange(), text }],
    () => null,
  )
  model.pushStackElement()
}
