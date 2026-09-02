import { createEffect, onCleanup, onMount } from 'solid-js'
import {
  createEditor,
  idOfModel,
  monaco,
  setProblems,
  switchModel,
} from '../../editor/monaco'
import type { Problem } from '../../ipc/bindings/Problem'
import type { Assist, Warning } from '../../lib/assist'
import type { Doc, DocId } from '../../lib/tweakspace'
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
  onEdit: (id: DocId, text: string) => void
  onSave: () => void
}) {
  let host: HTMLDivElement | undefined
  let editor: monaco.editor.IStandaloneCodeEditor | undefined

  onMount(() => {
    if (!host) return
    editor = createEditor(host, {
      minimap: { enabled: true },
      wordWrap: 'on',
      folding: true,
      bracketPairColorization: { enabled: true },
      insertSpaces: false,
    })
    editor.onDidChangeModelContent(() => {
      const model = editor?.getModel()
      if (!model) return
      props.onEdit(idOfModel(model), model.getValue())
    })
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () =>
      props.onSave(),
    )
    switchModel(editor, props.doc.id, props.doc.buffer)
    registerAssist(() => ({ assist: props.assist, kind: props.doc.kind }))
  })

  createEffect(() => {
    const { id, buffer } = props.doc
    if (editor) switchModel(editor, id, buffer)
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
