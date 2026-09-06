import { describe, expect, it } from 'vitest'
import type { Box } from '../../ipc/bindings/Box'
import { bounds, point, rect } from './geometry'
import { dirty, initial, reduce, type Action, type State } from './editor'

const square = rect(point(20, 20), point(60, 60))
const triangle: Box = {
  poly: [point(100, 100), point(150, 100), point(125, 140)],
}

function run(state: State, ...actions: Action[]): State {
  return actions.reduce(reduce, state)
}

/** A press at `from`, a move to `to`, a release there. */
function drag(from: [number, number], to: [number, number]): Action[] {
  return [
    { type: 'down', at: point(...from), tolerance: 3 },
    { type: 'move', at: point(...to) },
    { type: 'up', at: point(...to) },
  ]
}

describe('selecting and dragging', () => {
  const start = initial([square, triangle])

  it('a whole box rides along the border and keeps its shape', () => {
    const state = run(start, ...drag([40, 40], [400, 40]))
    expect(bounds(state.boxes[0]!)).toEqual({
      left: 160,
      top: 20,
      right: 200,
      bottom: 60,
    })
    expect(state.selected).toEqual({ box: 0, vertex: null })
    expect(dirty(state)).toBe(true)
  })

  it('a corner stops at the border', () => {
    // Corner 2 of the square is (60,60).
    const state = run(start, ...drag([60, 60], [999, 70]))
    expect(bounds(state.boxes[0]!)).toEqual({
      left: 20,
      top: 20,
      right: 200,
      bottom: 70,
    })
    expect(state.selected).toEqual({ box: 0, vertex: 2 })
  })

  it('a polygon edge carries both ends and stops at the border', () => {
    // Edge 0 of the triangle runs (100,100)-(150,100); grab its middle.
    const state = run(start, ...drag([125, 100], [125, -50]))
    expect(state.boxes[1]!.poly.slice(0, 2)).toEqual([
      point(100, 0),
      point(150, 0),
    ])
  })

  it('shows the drag as it goes and commits once, rounded', () => {
    const mid = run(
      start,
      { type: 'down', at: point(40, 40), tolerance: 3 },
      {
        type: 'move',
        at: point(50.3, 40),
      },
    )
    expect(bounds(mid.boxes[0]!).left).toBeCloseTo(30.3)
    expect(mid.past).toHaveLength(0)
    const done = reduce(mid, { type: 'up', at: point(50.3, 40) })
    expect(bounds(done.boxes[0]!).left).toBe(30)
    expect(done.past).toHaveLength(1)
  })

  it('a click that moves nothing is not an edit', () => {
    const state = run(start, ...drag([40, 40], [40, 40]))
    expect(state.boxes).toEqual(start.boxes)
    expect(dirty(state)).toBe(false)
    expect(state.selected).toEqual({ box: 0, vertex: null })
  })

  it('Escape puts the box back where it was', () => {
    const state = run(
      start,
      { type: 'down', at: point(40, 40), tolerance: 3 },
      { type: 'move', at: point(90, 90) },
      { type: 'cancel' },
    )
    expect(state.boxes).toEqual(start.boxes)
    expect(state.drag).toBeNull()
  })

  it('a press on nothing clears the selection', () => {
    const selected = run(start, { type: 'select', box: 1, vertex: null })
    expect(
      run(selected, { type: 'down', at: point(5, 190), tolerance: 3 }).selected,
    ).toBeNull()
  })
})

describe('drawing', () => {
  it('a rectangle from a drag, dropped when too small', () => {
    const tool = reduce(initial(), { type: 'tool', tool: 'rect' })
    const drawn = run(tool, ...drag([10, 10], [50.6, 80]))
    expect(drawn.boxes).toEqual([rect(point(10, 10), point(51, 80))])
    expect(drawn.selected).toEqual({ box: 0, vertex: null })
    expect(run(tool, ...drag([10, 10], [12, 80])).boxes).toEqual([])
  })

  it('a polygon by clicking, closed on the first vertex or by Enter', () => {
    const tool = reduce(initial(), { type: 'tool', tool: 'polygon' })
    const clicks = (...at: [number, number][]): Action[] =>
      at.map((p) => ({ type: 'down', at: point(...p), tolerance: 3 }))
    const closed = run(tool, ...clicks([10, 10], [90, 10], [50, 60], [11, 11]))
    expect(closed.boxes).toEqual([
      { poly: [point(10, 10), point(90, 10), point(50, 60)] },
    ])
    expect(closed.pending).toEqual([])

    const entered = run(tool, ...clicks([10, 10], [90, 10], [50, 60]), {
      type: 'close',
    })
    expect(entered.boxes).toHaveLength(1)

    // Two points cannot close; the third click near the first is just a vertex.
    const early = run(tool, ...clicks([10, 10], [90, 10], [11, 11]))
    expect(early.boxes).toEqual([])
    expect(early.pending).toHaveLength(3)

    expect(
      run(tool, ...clicks([10, 10], [90, 10]), { type: 'cancel' }).pending,
    ).toEqual([])
  })

  it('a lasso, simplified and clamped, dropped when it encloses nothing', () => {
    const tool = reduce(initial(), { type: 'tool', tool: 'lasso' })
    const path: [number, number][] = [
      [10, 10],
      [30, 10.3],
      [50, 10],
      [50, 30],
      [50, 50],
      [30, 50],
      [10, 50],
      [10, 30],
    ]
    const [first, ...rest] = path
    const state = run(
      tool,
      { type: 'down', at: point(...first!), tolerance: 3 },
      ...rest.map((p): Action => ({ type: 'move', at: point(...p) })),
      { type: 'up', at: point(-20, 30) },
    )
    // The tremor at (30,10.3) and the points on straight runs go; the corners
    // stay, the jut to the clamped (0,30) included.
    expect(state.boxes).toEqual([
      {
        poly: [
          point(10, 10),
          point(50, 10),
          point(50, 50),
          point(10, 50),
          point(10, 30),
          point(0, 30),
        ],
      },
    ])

    const nothing = run(tool, ...drag([10, 10], [12, 12]))
    expect(nothing.boxes).toEqual([])
  })
})

describe('the boxes list', () => {
  const start = initial([square, triangle])

  it('adds Chobby’s centre box and selects it', () => {
    const state = reduce(start, { type: 'add' })
    expect(state.boxes).toHaveLength(3)
    expect(bounds(state.boxes[2]!)).toEqual({
      left: 66,
      top: 66,
      right: 133,
      bottom: 133,
    })
    expect(state.selected).toEqual({ box: 2, vertex: null })
  })

  it('deletes the selected box, or the selected vertex when it can', () => {
    const noSquare = run(
      start,
      { type: 'select', box: 0, vertex: null },
      { type: 'delete' },
    )
    expect(noSquare.boxes).toEqual([triangle])
    expect(noSquare.selected).toBeNull()

    // A triangle keeps its three vertices.
    const kept = run(
      start,
      { type: 'select', box: 1, vertex: 0 },
      { type: 'delete' },
    )
    expect(kept.boxes).toEqual(start.boxes)

    const grown = reduce(start, {
      type: 'insert',
      box: 1,
      edge: 0,
      at: point(125, 90),
    })
    expect(grown.boxes[1]!.poly).toHaveLength(4)
    expect(grown.selected).toEqual({ box: 1, vertex: 1 })
    const shrunk = reduce(grown, { type: 'delete' })
    expect(shrunk.boxes[1]).toEqual(triangle)
  })

  it('reorders, which is what changes the ally team a box belongs to', () => {
    const state = reduce(start, { type: 'reorder', from: 1, to: 0 })
    expect(state.boxes).toEqual([triangle, square])
    expect(state.selected).toEqual({ box: 0, vertex: null })
  })

  it('curves the selected vertex, turning a rectangle into a polygon', () => {
    const state = run(
      start,
      { type: 'select', box: 0, vertex: 1 },
      { type: 'strength', value: 0.5 },
    )
    expect(state.boxes[0]!.poly).toHaveLength(4)
    expect(state.boxes[0]!.poly[1]).toEqual(point(60, 20, 0.5))
  })

  it('simplifies the selected polygon and leaves a triangle alone', () => {
    const noisy: Box = {
      poly: [
        point(0, 0),
        point(50, 0.2),
        point(100, 0),
        point(100, 100),
        point(0, 100),
      ],
    }
    const state = run(
      initial([noisy]),
      { type: 'select', box: 0, vertex: null },
      { type: 'simplify', epsilon: 1.5 },
    )
    expect(state.boxes[0]!.poly).toHaveLength(4)
    const selected = reduce(start, { type: 'select', box: 1, vertex: null })
    expect(reduce(selected, { type: 'simplify', epsilon: 5 })).toBe(selected)
  })
})

describe('undo', () => {
  const start = initial([square])

  it('steps back through commits and forward again', () => {
    const edited = run(start, { type: 'add' }, ...drag([40, 40], [45, 40]))
    expect(edited.boxes).toHaveLength(2)
    expect(bounds(edited.boxes[0]!).left).toBe(25)

    const once = reduce(edited, { type: 'undo' })
    expect(bounds(once.boxes[0]!).left).toBe(20)
    expect(once.boxes).toHaveLength(2)

    const twice = reduce(once, { type: 'undo' })
    expect(twice.boxes).toEqual([square])
    expect(reduce(twice, { type: 'undo' })).toBe(twice)

    const redone = reduce(twice, { type: 'redo' })
    expect(redone.boxes).toHaveLength(2)
    expect(dirty(redone)).toBe(true)
  })

  it('a new edit after undo forgets what was undone', () => {
    const state = run(start, { type: 'add' }, { type: 'undo' }, { type: 'add' })
    expect(state.future).toEqual([])
  })

  it('applying clears the stack; loading replaces the draft', () => {
    const applied = run(start, { type: 'add' }, { type: 'applied' })
    expect(dirty(applied)).toBe(false)
    expect(applied.boxes).toHaveLength(2)

    const loaded = reduce(applied, { type: 'load', boxes: [triangle] })
    expect(loaded.boxes).toEqual([triangle])
    expect(loaded.selected).toBeNull()
  })
})
