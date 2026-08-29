// Only the editor core and Lua are pulled in; Monaco 0.56 keeps each language
// behind `languages/definitions/<id>/register`, so the rest never ships. Its
// exports map rewrites subpaths, so these are `monaco-editor/<path under esm/vs>`.
import * as monaco from 'monaco-editor/editor/editor.api'
import 'monaco-editor/languages/definitions/lua/register'
import EditorWorker from 'monaco-editor/editor/editor.worker?worker'

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment
  }
}

// Lua has no language service, so the plain editor worker is the only one needed.
self.MonacoEnvironment = { getWorker: () => new EditorWorker() }

export { monaco }

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

export function createEditor(
  host: HTMLElement,
  value: string,
): monaco.editor.IStandaloneCodeEditor {
  return monaco.editor.create(host, { ...OPTIONS, value })
}

export function createDiffEditor(
  host: HTMLElement,
  original: string,
  modified: string,
): monaco.editor.IStandaloneDiffEditor {
  const editor = monaco.editor.createDiffEditor(host, {
    ...OPTIONS,
    readOnly: true,
    renderSideBySide: true,
  })
  editor.setModel({
    original: monaco.editor.createModel(original, 'lua'),
    modified: monaco.editor.createModel(modified, 'lua'),
  })
  return editor
}

/**
 * Replaces the buffer through the edit stack rather than `setValue`, so a view
 * switch that clobbers work is still one Ctrl+Z away.
 */
export function seed(
  editor: monaco.editor.IStandaloneCodeEditor,
  text: string,
): void {
  const model = editor.getModel()
  if (!model || model.getValue() === text) return
  editor.pushUndoStop()
  editor.executeEdits('seed', [{ range: model.getFullModelRange(), text }])
  editor.pushUndoStop()
}

export function disposeDiff(editor: monaco.editor.IStandaloneDiffEditor): void {
  const model = editor.getModel()
  editor.dispose()
  model?.original.dispose()
  model?.modified.dispose()
}
