/**
 * The start-box editor's state, and what each gesture does to it.
 *
 * A pure reducer: the component turns pointer events into `down`, `move` and
 * `up` in map units and draws whatever comes back. That is what lets a whole
 * drag -- including one that ends off the map -- be a unit test rather than
 * something to try by hand.
 *
 * The draft is local. Nothing leaves the reducer until the caller reads
 * `boxes` and sends them; `past`/`future` are the undo stack over committed
 * edits, a drag in flight is `drag`, and `snapshot` is where the boxes were
 * when it began, so Escape can put them back.
 */

import type { Box } from '../../ipc/bindings/Box'
import type { Point } from '../../ipc/bindings/Point'
import {
  centreBox,
  corners,
  hitTest,
  insertVertex,
  lassoBox,
  moveEdge,
  moveVertex,
  point,
  rectFrom,
  removeVertex,
  round,
  setStrength,
  simplify,
  translateClamped,
  type Hit,
} from './geometry'

export type Tool = 'select' | 'rect' | 'polygon' | 'lasso'

export type Selection = { box: number; vertex: number | null } | null

export type Drag =
  | { kind: 'vertex'; box: number; index: number }
  | { kind: 'edge'; box: number; index: number; from: Point; before: Box }
  | { kind: 'box'; box: number; from: Point; before: Box }
  | { kind: 'rect'; from: Point; to: Point }
  | { kind: 'lasso'; points: Point[] }
  | null

export type State = {
  boxes: Box[]
  selected: Selection
  tool: Tool
  drag: Drag
  /** The boxes as they were when the drag in flight began. */
  snapshot: Box[] | null
  /** Vertices placed so far with the polygon tool. */
  pending: Point[]
  past: Box[][]
  future: Box[][]
}

export type Action =
  | { type: 'load'; boxes: Box[] }
  | { type: 'tool'; tool: Tool }
  | { type: 'down'; at: Point; tolerance: number }
  | { type: 'move'; at: Point }
  | { type: 'up'; at: Point }
  /** Escape: abandon the drag or the polygon being placed. */
  | { type: 'cancel' }
  /** Enter: close the polygon being placed. */
  | { type: 'close' }
  | { type: 'add' }
  | { type: 'delete' }
  | { type: 'select'; box: number; vertex: number | null }
  | { type: 'insert'; box: number; edge: number; at: Point }
  | { type: 'strength'; value: number }
  | { type: 'reorder'; from: number; to: number }
  | { type: 'simplify'; epsilon: number }
  | { type: 'undo' }
  | { type: 'redo' }
  /** The draft went out; there is nothing left to undo. */
  | { type: 'applied' }

export function initial(boxes: Box[] = []): State {
  return {
    boxes,
    selected: null,
    tool: 'select',
    drag: null,
    snapshot: null,
    pending: [],
    past: [],
    future: [],
  }
}

/** Whether the draft differs from what was loaded or last applied. */
export function dirty(state: State): boolean {
  return state.past.length > 0
}

/** Radius, in map units, within which a click on the first vertex closes a polygon. */
export const CLOSE_RADIUS = 4

function commit(
  state: State,
  boxes: Box[],
  selected: Selection = state.selected,
): State {
  return {
    ...state,
    boxes,
    selected,
    drag: null,
    snapshot: null,
    past: [...state.past, state.boxes],
    future: [],
  }
}

function replace(boxes: Box[], index: number, box: Box): Box[] {
  return boxes.map((held, i) => (i === index ? box : held))
}

function down(state: State, at: Point, tolerance: number): State {
  switch (state.tool) {
    case 'select':
      return grab(state, hitTest(state.boxes, at, tolerance), at)
    case 'rect':
      return {
        ...state,
        selected: null,
        drag: { kind: 'rect', from: at, to: at },
      }
    case 'lasso':
      return { ...state, selected: null, drag: { kind: 'lasso', points: [at] } }
    case 'polygon':
      return place(state, at)
  }
}

function grab(state: State, hit: Hit, at: Point): State {
  if (hit === null) return { ...state, selected: null }
  const box = state.boxes[hit.box]
  if (box === undefined) return state
  const snapshot = state.boxes
  switch (hit.kind) {
    case 'vertex':
      return {
        ...state,
        selected: { box: hit.box, vertex: hit.index },
        drag: { kind: 'vertex', box: hit.box, index: hit.index },
        snapshot,
      }
    case 'edge':
      return {
        ...state,
        selected: { box: hit.box, vertex: null },
        drag: {
          kind: 'edge',
          box: hit.box,
          index: hit.index,
          from: at,
          before: box,
        },
        snapshot,
      }
    case 'inside':
      return {
        ...state,
        selected: { box: hit.box, vertex: null },
        drag: { kind: 'box', box: hit.box, from: at, before: box },
        snapshot,
      }
  }
}

function place(state: State, at: Point): State {
  const first = state.pending[0]
  if (
    first !== undefined &&
    state.pending.length >= 3 &&
    Math.hypot(first.x - at.x, first.y - at.y) <= CLOSE_RADIUS
  ) {
    return close(state)
  }
  return { ...state, selected: null, pending: [...state.pending, at] }
}

function close(state: State): State {
  if (state.pending.length < 3) return { ...state, pending: [] }
  const box = round({ poly: state.pending.map((p) => point(p.x, p.y)) })
  const boxes = [...state.boxes, box]
  return commit({ ...state, pending: [] }, boxes, {
    box: boxes.length - 1,
    vertex: null,
  })
}

function move(state: State, at: Point): State {
  const drag = state.drag
  if (drag === null) return state
  switch (drag.kind) {
    case 'vertex': {
      const box = state.boxes[drag.box]
      if (box === undefined) return state
      return {
        ...state,
        boxes: replace(state.boxes, drag.box, moveVertex(box, drag.index, at)),
      }
    }
    case 'edge':
      return {
        ...state,
        boxes: replace(
          state.boxes,
          drag.box,
          moveEdge(
            drag.before,
            drag.index,
            at.x - drag.from.x,
            at.y - drag.from.y,
          ),
        ),
      }
    case 'box':
      return {
        ...state,
        boxes: replace(
          state.boxes,
          drag.box,
          translateClamped(drag.before, at.x - drag.from.x, at.y - drag.from.y),
        ),
      }
    case 'rect':
      return { ...state, drag: { ...drag, to: at } }
    case 'lasso':
      return { ...state, drag: { ...drag, points: [...drag.points, at] } }
  }
}

function up(state: State, at: Point): State {
  const drag = state.drag
  if (drag === null) return state
  switch (drag.kind) {
    case 'vertex':
    case 'edge':
    case 'box': {
      const base = state.snapshot ?? state.boxes
      const box = move(state, at).boxes[drag.box]
      const before = base[drag.box]
      if (box === undefined || before === undefined) {
        return { ...state, boxes: base, drag: null, snapshot: null }
      }
      const rounded = round(box)
      // A click that moved nothing is not an edit, and must not be undoable.
      if (sameBox(rounded, before))
        return { ...state, boxes: base, drag: null, snapshot: null }
      return commit({ ...state, boxes: base }, replace(base, drag.box, rounded))
    }
    case 'rect': {
      const box = rectFrom(drag.from, at)
      if (box === null) return { ...state, drag: null }
      const boxes = [...state.boxes, round(box)]
      return commit(state, boxes, { box: boxes.length - 1, vertex: null })
    }
    case 'lasso': {
      const box = lassoBox([...drag.points, at])
      if (box === null) return { ...state, drag: null }
      const boxes = [...state.boxes, round(box)]
      return commit(state, boxes, { box: boxes.length - 1, vertex: null })
    }
  }
}

function sameBox(a: Box, b: Box): boolean {
  if (a.poly.length !== b.poly.length) return false
  return a.poly.every((p, i) => {
    const q = b.poly[i]
    return (
      q !== undefined &&
      p.x === q.x &&
      p.y === q.y &&
      (p.strength ?? null) === (q.strength ?? null)
    )
  })
}

function cancel(state: State): State {
  if (state.drag !== null) {
    return {
      ...state,
      boxes: state.snapshot ?? state.boxes,
      drag: null,
      snapshot: null,
    }
  }
  return { ...state, pending: [] }
}

function remove(state: State): State {
  const selected = state.selected
  if (selected === null) return state
  const box = state.boxes[selected.box]
  if (box === undefined) return state
  if (selected.vertex !== null) {
    const smaller = removeVertex(box, selected.vertex)
    if (smaller === null) return state
    return commit(state, replace(state.boxes, selected.box, smaller), {
      box: selected.box,
      vertex: null,
    })
  }
  return commit(
    state,
    state.boxes.filter((_, i) => i !== selected.box),
    null,
  )
}

function reorder(state: State, from: number, to: number): State {
  if (
    from === to ||
    state.boxes[from] === undefined ||
    state.boxes[to] === undefined
  )
    return state
  const boxes = [...state.boxes]
  const [moved] = boxes.splice(from, 1)
  if (moved === undefined) return state
  boxes.splice(to, 0, moved)
  return commit(state, boxes, { box: to, vertex: null })
}

export function reduce(state: State, action: Action): State {
  switch (action.type) {
    case 'load':
      return { ...initial(action.boxes), tool: state.tool }
    case 'tool':
      return {
        ...state,
        tool: action.tool,
        pending: [],
        drag: null,
        snapshot: null,
      }
    case 'down':
      return down(state, action.at, action.tolerance)
    case 'move':
      return move(state, action.at)
    case 'up':
      return up(state, action.at)
    case 'cancel':
      return cancel(state)
    case 'close':
      return close(state)
    case 'add': {
      const boxes = [...state.boxes, centreBox()]
      return commit(state, boxes, { box: boxes.length - 1, vertex: null })
    }
    case 'delete':
      return remove(state)
    case 'select':
      return { ...state, selected: { box: action.box, vertex: action.vertex } }
    case 'insert': {
      const box = state.boxes[action.box]
      if (box === undefined) return state
      const grown = round(insertVertex(box, action.edge, action.at))
      // The new vertex sits after corner `edge`; on a rectangle that has just
      // become four corners the index still holds.
      const index = (action.edge % corners(box).length) + 1
      return commit(state, replace(state.boxes, action.box, grown), {
        box: action.box,
        vertex: index,
      })
    }
    case 'strength': {
      const selected = state.selected
      if (selected === null || selected.vertex === null) return state
      const box = state.boxes[selected.box]
      if (box === undefined) return state
      return commit(
        state,
        replace(
          state.boxes,
          selected.box,
          round(setStrength(box, selected.vertex, action.value)),
        ),
      )
    }
    case 'reorder':
      return reorder(state, action.from, action.to)
    case 'simplify': {
      const selected = state.selected
      if (selected === null) return state
      const box = state.boxes[selected.box]
      if (box === undefined || box.poly.length <= 3) return state
      const poly = simplify(box.poly, action.epsilon)
      if (poly.length === box.poly.length) return state
      return commit(state, replace(state.boxes, selected.box, { poly }), {
        box: selected.box,
        vertex: null,
      })
    }
    case 'undo': {
      const previous = state.past[state.past.length - 1]
      if (previous === undefined) return state
      return {
        ...state,
        boxes: previous,
        selected: null,
        drag: null,
        snapshot: null,
        pending: [],
        past: state.past.slice(0, -1),
        future: [state.boxes, ...state.future],
      }
    }
    case 'redo': {
      const [next, ...rest] = state.future
      if (next === undefined) return state
      return {
        ...state,
        boxes: next,
        selected: null,
        drag: null,
        snapshot: null,
        past: [...state.past, state.boxes],
        future: rest,
      }
    }
    case 'applied':
      return { ...state, past: [], future: [] }
  }
}
