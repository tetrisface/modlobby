import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  on,
  onCleanup,
  onMount,
} from 'solid-js'
import type { Box } from '../ipc/bindings/Box'
import type { Point } from '../ipc/bindings/Point'
import { api, describeError } from '../ipc/client'
import {
  BOX_KEYS,
  boxSignature,
  centre,
  extent,
  isBoxKey,
  outline,
  type Poly,
} from '../lib/boxes'
import { mapPicture } from '../lib/maps'
import {
  dirty,
  initial,
  reduce,
  type Action,
  type State,
  type Tool,
} from '../lib/startbox/editor'
import { SIZE, bounds, corners, hitTest, point } from '../lib/startbox/geometry'
import {
  GRAB_PIXELS,
  HANDLE_STROKE,
  handleSize,
  labelSize,
} from '../lib/startbox/scale'
import { ring } from '../lib/startbox/spline'
import { pushNotice } from '../store/chat'
import { useRoom } from '../views/room/model'

/** The modoption a drawn arrangement is sent as; `"0"` clears it. */
const OVERRIDE = BOX_KEYS[0]

/** One notch of the wheel over a corner, in curvature. */
const WHEEL_STEP = 0.1

/** How long the boxes may be still before the wire size is asked for again. */
const ENCODE_DEBOUNCE_MS = 120

const TOOLS: ReadonlyArray<{ tool: Tool; label: string; hint: string }> = [
  { tool: 'select', label: 'Select', hint: 'Drag a box, a corner or an edge' },
  { tool: 'rect', label: 'Rectangle', hint: 'Drag from corner to corner' },
  {
    tool: 'polygon',
    label: 'Polygon',
    hint: 'Click the corners; click the first again or press Enter to close',
  },
  {
    tool: 'lasso',
    label: 'Lasso',
    hint: 'Draw round the area while holding the button',
  },
]

/**
 * The map, large, with the start boxes drawn on it and the means to move them.
 *
 * Edits are a local draft: nothing reaches the room until Apply sends the
 * whole arrangement as one `!bSet mapmetadata_startbox_override`, which SPADS
 * runs for the boss and puts to a vote for anyone else. The room's own boxes
 * keep showing in the minimap behind this sheet until that echo lands.
 *
 * The geometry and the gestures live in `lib/startbox`; this component owns
 * the pointer choreography, the picture and the buttons.
 */
export function MapEditor(props: {
  mapName: string
  teams: number
  onClose: () => void
}) {
  const [state, setState] = createSignal<State>(initial())
  const dispatch = (action: Action) => setState((held) => reduce(held, action))

  let svg: SVGSVGElement | undefined
  let card: HTMLDivElement | undefined

  const room = useRoom()
  const me = () => room.me()
  const boss = () => me() !== null && room.my()?.boss === me()
  /**
   * SPADS refuses `!bSet` from a spectator (`commands.rs` `set_option`). In a
   * room with no host there is nobody to refuse it.
   */
  const editable = createMemo(() => {
    if (!room.caps.spads) return true
    const name = me()
    return name !== null && room.users()[name]?.battleStatus?.player === true
  })

  const tags = () => room.my()?.scriptTags
  const overrideSet = () => {
    const held = tags()?.[`game/modoptions/${OVERRIDE}`] ?? ''
    return held !== '' && held !== '0'
  }

  /** Who last moved the boxes, from the host's announcement when there was one. */
  const lastMover = () => {
    const changes = room.my()?.history ?? []
    for (let i = changes.length - 1; i >= 0; i--) {
      const change = changes[i]
      if (change !== undefined && isBoxKey(change.key))
        return change.by ?? 'someone'
    }
    return 'someone'
  }

  async function load() {
    try {
      const held = await room.io.currentArrangement(props.teams)
      dispatch({ type: 'load', boxes: held?.arrangement.startboxes ?? [] })
    } catch (error) {
      pushNotice('warning', `start boxes: ${describeError(error)}`)
    }
    setStale(false)
  }

  /**
   * The room's boxes moved under an open draft. A clean draft follows them; a
   * dirty one is somebody's work and is kept until they say otherwise.
   */
  const [stale, setStale] = createSignal(false)
  createEffect(
    on(
      () => boxSignature(tags()),
      () => {
        if (dirty(state())) setStale(true)
        else void load()
      },
      { defer: true },
    ),
  )

  onMount(() => {
    void load()
    card?.focus()
    watchSize()
  })

  /**
   * The map's own arrangement for this many teams, drawn faintly underneath:
   * what the room falls back to, and what a custom box is placed against.
   */
  const [ghost] = createResource(
    () =>
      [tags()?.[`game/modoptions/${BOX_KEYS[1]}`] ?? '', props.teams] as const,
    ([blob, teams]) =>
      blob === '' ? null : api.decodeBoxes(blob, teams).catch(() => null),
  )

  /** The draft as the wire would carry it, asked for once the boxes hold still. */
  const [settled, setSettled] = createSignal<Box[]>([])
  createEffect(
    on(
      () => state().boxes,
      (boxes) => {
        const timer = setTimeout(() => setSettled(boxes), ENCODE_DEBOUNCE_MS)
        onCleanup(() => clearTimeout(timer))
      },
    ),
  )
  const [encoded] = createResource(settled, async (boxes) => {
    if (boxes.length === 0) return null
    try {
      return await api.encodeBoxes({ startboxes: boxes })
    } catch (error) {
      return { error: describeError(error) }
    }
  })
  /** The blob and its limit, once Rust has answered and could read the shape. */
  const wire = () => {
    const held = encoded()
    return held && 'value' in held ? held : null
  }
  /**
   * Too long to send. The limit is the room's chat cap, so it applies only
   * where the boxes travel as a chat line: a room of your own reads them out
   * of a start script, which has no such thing.
   */
  const over = () => {
    const held = wire()
    return room.caps.spads && held !== null && held.value.length > held.limit
  }
  const unreadable = () => {
    const held = encoded()
    return held && 'error' in held ? held.error : null
  }
  const sendable = createMemo(
    () =>
      editable() &&
      wire() !== null &&
      !over() &&
      state().boxes.length > 0 &&
      state().drag === null,
  )

  /** What applying them will have done, which depends on who decides here. */
  const sent = () => {
    if (!room.caps.spads) return 'start boxes set'
    if (boss()) return 'start boxes sent to the host'
    return 'start boxes proposed as a vote'
  }

  /** The same question before the fact, in the header. */
  const applyNote = () => {
    if (!editable()) return 'read-only · spectator'
    if (!room.caps.spads) return 'Apply sets them'
    return boss() ? 'Apply sends to the host' : 'Apply proposes a vote'
  }

  const [sending, setSending] = createSignal(false)
  async function send(value: string) {
    setSending(true)
    try {
      await room.io.setOption(OVERRIDE, value)
      dispatch({ type: 'applied' })
      pushNotice('info', sent())
      // Sending is the end of the sheet's job. It leaves through the ordinary
      // door: the draft is no longer unsaved work, so nothing is asked.
      close()
    } catch (error) {
      pushNotice('warning', `start boxes: ${describeError(error)}`)
    } finally {
      setSending(false)
    }
  }

  function apply() {
    const held = wire()
    if (held !== null) void send(held.value)
  }

  /** A close that would lose work asks first. */
  const [leaving, setLeaving] = createSignal(false)
  function close() {
    if (dirty(state()) && !leaving()) {
      setLeaving(true)
      return
    }
    props.onClose()
  }

  // --- the map's shape --------------------------------------------------

  /**
   * The drawn size of the map in screen pixels.
   *
   * The 0-200 space is stretched onto whatever shape the map is, which is what
   * makes a box land where the game will put it and what makes everything else
   * come out wrong: on a 2:1 map a digit is drawn twice as wide as it is tall
   * and a round handle is an ellipse. Knowing the two scales is enough to undo
   * that for the few things that have a shape of their own.
   */
  const [frame, setFrame] = createSignal(
    { width: SIZE, height: SIZE },
    { equals: (a, b) => a.width === b.width && a.height === b.height },
  )

  /**
   * An anchor whose children are drawn in screen pixels at `p`, upright,
   * whatever the map's shape has done to the space around them.
   */
  const upright = (p: { x: number; y: number }) => {
    const { width, height } = frame()
    return `translate(${p.x} ${p.y}) scale(${SIZE / width} ${SIZE / height})`
  }

  /** Follows the map picture as the sheet and the window resize. */
  function watchSize() {
    const box = svg
    if (box === undefined) return
    const read = () => {
      const rect = box.getBoundingClientRect()
      if (rect.width > 0 && rect.height > 0)
        setFrame({ width: rect.width, height: rect.height })
    }
    read()
    if (typeof ResizeObserver === 'undefined') return
    const watch = new ResizeObserver(read)
    watch.observe(box)
    onCleanup(() => watch.disconnect())
  }

  // --- the pointer, in map units ---------------------------------------

  function place(event: PointerEvent | MouseEvent): {
    at: Point
    tolerance: number
  } {
    const rect = svg?.getBoundingClientRect()
    const width = rect?.width || 1
    const height = rect?.height || 1
    const at = point(
      ((event.clientX - (rect?.left ?? 0)) / width) * SIZE,
      ((event.clientY - (rect?.top ?? 0)) / height) * SIZE,
    )
    return { at, tolerance: (GRAB_PIXELS * SIZE) / Math.min(width, height) }
  }

  function down(event: PointerEvent) {
    if (event.button !== 0 || !editable()) return
    event.preventDefault()
    svg?.setPointerCapture?.(event.pointerId)
    dispatch({ type: 'down', ...place(event) })
  }

  function move(event: PointerEvent) {
    if (state().drag === null) return
    dispatch({ type: 'move', at: place(event).at })
  }

  function up(event: PointerEvent) {
    if (state().drag === null) return
    dispatch({ type: 'up', at: place(event).at })
  }

  /** A double click on an edge grows a corner there. */
  function grow(event: MouseEvent) {
    if (!editable() || state().tool !== 'select') return
    const { at, tolerance } = place(event)
    const hit = hitTest(state().boxes, at, tolerance)
    if (hit?.kind === 'edge')
      dispatch({ type: 'insert', box: hit.box, edge: hit.index, at })
  }

  /** The wheel over a corner bends the edges that meet there. */
  function wheel(event: WheelEvent) {
    if (!editable()) return
    const { at, tolerance } = place(event)
    const hit = hitTest(state().boxes, at, tolerance)
    if (hit?.kind !== 'vertex') return
    event.preventDefault()
    const held =
      corners(state().boxes[hit.box] ?? { poly: [] })[hit.index]?.strength ?? 0
    dispatch({ type: 'select', box: hit.box, vertex: hit.index })
    dispatch({
      type: 'strength',
      value: held + (event.deltaY < 0 ? WHEEL_STEP : -WHEEL_STEP),
    })
  }

  function key(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null
    if (target?.tagName === 'INPUT') return
    const busy = state().drag !== null || state().pending.length > 0
    switch (event.key) {
      case 'Escape':
        event.preventDefault()
        if (leaving()) setLeaving(false)
        else if (busy) dispatch({ type: 'cancel' })
        else close()
        return
      case 'Enter':
        if (state().tool === 'polygon' && editable()) {
          event.preventDefault()
          dispatch({ type: 'close' })
        }
        return
      case 'Delete':
      case 'Backspace':
        if (editable()) {
          event.preventDefault()
          dispatch({ type: 'delete' })
        }
        return
      case 'z':
      case 'Z':
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault()
          dispatch({ type: event.shiftKey ? 'redo' : 'undo' })
        }
        return
      case 'y':
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault()
          dispatch({ type: 'redo' })
        }
        return
    }
  }

  // --- what is drawn ----------------------------------------------------

  const selectedBox = () => {
    const selected = state().selected
    return selected === null ? undefined : state().boxes[selected.box]
  }
  const selectedVertex = () => {
    const selected = state().selected
    const box = selectedBox()
    if (selected === null || selected.vertex === null || box === undefined)
      return null
    return corners(box)[selected.vertex] ?? null
  }

  const draft = (): Poly | null => {
    const drag = state().drag
    if (drag?.kind === 'rect') {
      return [
        [drag.from.x, drag.from.y],
        [drag.to.x, drag.from.y],
        [drag.to.x, drag.to.y],
        [drag.from.x, drag.to.y],
      ]
    }
    if (drag?.kind === 'lasso')
      return drag.points.map((p) => [p.x, p.y] as const)
    if (state().pending.length > 0)
      return state().pending.map((p) => [p.x, p.y] as const)
    return null
  }

  const shortfall = () => Math.max(0, props.teams - state().boxes.length)

  return (
    <div class='sheet' onMouseDown={close}>
      <div
        ref={card}
        class='sheet-card map-editor'
        tabIndex={0}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={key}
      >
        <header class='ed-head'>
          <h2>Start boxes · {props.mapName}</h2>
          <span class='note'>{applyNote()}</span>
          <button type='button' onClick={close}>
            Close
          </button>
        </header>

        <Show when={stale()}>
          <div class='ed-banner' role='status'>
            The boxes in the room were changed by {lastMover()} while you were
            editing.
            <button type='button' onClick={() => void load()}>
              Reload
            </button>
          </div>
        </Show>
        <Show when={leaving()}>
          <div class='ed-banner warn' role='alertdialog'>
            Unsaved changes.
            <button type='button' onClick={props.onClose}>
              Discard
            </button>
            <button type='button' onClick={() => setLeaving(false)}>
              Keep editing
            </button>
          </div>
        </Show>

        <div class='ed-body'>
          <div
            class='ed-map'
            classList={{ drawing: state().tool !== 'select' }}
          >
            <div class='ed-frame'>
              <Show
                when={mapPicture(props.mapName)}
                fallback={<div class='ed-blank' />}
              >
                {(url) => <img src={url()} alt='' draggable={false} />}
              </Show>
              <svg
                ref={svg}
                viewBox={`0 0 ${SIZE} ${SIZE}`}
                preserveAspectRatio='none'
                role='img'
                onPointerDown={down}
                onPointerMove={move}
                onPointerUp={up}
                onPointerCancel={up}
                onDblClick={grow}
                onWheel={wheel}
              >
                <title>Start boxes on {props.mapName}</title>
                <For each={ghost() ?? []}>
                  {(poly, index) => (
                    <g class='ed-ghost'>
                      <path d={outline(poly)} />
                      <text
                        transform={upright(centre(poly))}
                        dominant-baseline='central'
                        style={{
                          'font-size': `${labelSize(frame(), extent(poly))}px`,
                        }}
                      >
                        {index() + 1}
                      </text>
                    </g>
                  )}
                </For>
                <For each={state().boxes}>
                  {(box, index) => {
                    const shape = createMemo(() => ring(box))
                    return (
                      <g
                        class='ed-box'
                        classList={{ on: state().selected?.box === index() }}
                      >
                        <path d={outline(shape())} />
                        <text
                          transform={upright(centre(shape()))}
                          dominant-baseline='central'
                          style={{
                            'font-size': `${labelSize(
                              frame(),
                              extent(shape()),
                            )}px`,
                          }}
                        >
                          {index() + 1}
                        </text>
                      </g>
                    )
                  }}
                </For>
                <Show when={selectedBox()}>
                  {(box) => (
                    <g class='ed-handles'>
                      <For each={corners(box())}>
                        {(p, index) => (
                          <circle
                            transform={upright(p)}
                            r={handleSize(frame(), bounds(box()))}
                            stroke-width={HANDLE_STROKE}
                            classList={{
                              on: state().selected?.vertex === index(),
                              curved: (p.strength ?? 0) > 0,
                            }}
                          />
                        )}
                      </For>
                    </g>
                  )}
                </Show>
                <Show when={draft()}>
                  {(poly) => (
                    <g class='ed-draft'>
                      <path d={outline(poly())} />
                      <Show when={state().pending[0]}>
                        {(first) => (
                          <circle cx={first().x} cy={first().y} r={4} />
                        )}
                      </Show>
                    </g>
                  )}
                </Show>
              </svg>
            </div>
          </div>

          <aside class='ed-side'>
            <div class='ed-tools' role='toolbar' aria-label='Tools'>
              <For each={TOOLS}>
                {(entry) => (
                  <button
                    type='button'
                    classList={{ on: state().tool === entry.tool }}
                    aria-pressed={state().tool === entry.tool}
                    title={entry.hint}
                    disabled={!editable()}
                    onClick={() => dispatch({ type: 'tool', tool: entry.tool })}
                  >
                    {entry.label}
                  </button>
                )}
              </For>
            </div>
            <p class='muted ed-hint'>
              {TOOLS.find((entry) => entry.tool === state().tool)?.hint}
            </p>

            <div class='ed-actions'>
              <button
                type='button'
                disabled={!editable()}
                onClick={() => dispatch({ type: 'add' })}
              >
                Add box
              </button>
              <button
                type='button'
                disabled={!editable() || state().selected === null}
                onClick={() => dispatch({ type: 'delete' })}
              >
                Delete
              </button>
              <button
                type='button'
                disabled={state().past.length === 0}
                title='Ctrl+Z'
                onClick={() => dispatch({ type: 'undo' })}
              >
                Undo
              </button>
              <button
                type='button'
                disabled={state().future.length === 0}
                title='Ctrl+Shift+Z'
                onClick={() => dispatch({ type: 'redo' })}
              >
                Redo
              </button>
            </div>

            <Show when={selectedVertex()}>
              {(vertex) => (
                <label class='ed-strength'>
                  Curve at this corner
                  <input
                    type='range'
                    min={0}
                    max={1}
                    step={0.05}
                    value={vertex().strength ?? 0}
                    disabled={!editable()}
                    onInput={(event) =>
                      dispatch({
                        type: 'strength',
                        value: Number(event.currentTarget.value),
                      })
                    }
                  />
                  <span class='mono'>
                    {(vertex().strength ?? 0).toFixed(2)}
                  </span>
                </label>
              )}
            </Show>

            <ol class='ed-list' aria-label='Boxes'>
              <For each={state().boxes}>
                {(box, index) => (
                  <li classList={{ on: state().selected?.box === index() }}>
                    <button
                      type='button'
                      class='ed-pick'
                      onClick={() =>
                        dispatch({ type: 'select', box: index(), vertex: null })
                      }
                    >
                      <span class='n'>{index() + 1}</span>
                      <span class='muted'>
                        {box.poly.length === 2
                          ? 'rectangle'
                          : `${box.poly.length} corners`}
                      </span>
                    </button>
                    <button
                      type='button'
                      title='Earlier team'
                      disabled={!editable() || index() === 0}
                      onClick={() =>
                        dispatch({
                          type: 'reorder',
                          from: index(),
                          to: index() - 1,
                        })
                      }
                    >
                      ↑
                    </button>
                    <button
                      type='button'
                      title='Later team'
                      disabled={
                        !editable() || index() === state().boxes.length - 1
                      }
                      onClick={() =>
                        dispatch({
                          type: 'reorder',
                          from: index(),
                          to: index() + 1,
                        })
                      }
                    >
                      ↓
                    </button>
                  </li>
                )}
              </For>
            </ol>

            <Show when={shortfall() > 0 && state().boxes.length > 0}>
              <p class='ed-warn'>
                {shortfall()} {shortfall() === 1 ? 'team has' : 'teams have'} no
                box: the game will ignore this override and use the map's boxes.
              </p>
            </Show>
            <Show when={unreadable()}>
              {(why) => <p class='ed-warn'>{why()}</p>}
            </Show>

            <div class='ed-budget' classList={{ over: over() }}>
              <Show
                when={wire()}
                fallback={<span class='muted'>nothing to send</span>}
              >
                {(held) => (
                  <>
                    <span class='mono'>
                      {held().value.length}
                      <Show when={room.caps.spads}> / {held().limit}</Show>
                    </span>
                    <span class='muted'>
                      {room.caps.spads
                        ? ' characters on the wire'
                        : ' characters'}
                    </span>
                    <Show when={over()}>
                      <button
                        type='button'
                        disabled={!editable() || state().selected === null}
                        title='Drop corners that barely change the selected box'
                        onClick={() =>
                          dispatch({ type: 'simplify', epsilon: 3 })
                        }
                      >
                        Simplify
                      </button>
                    </Show>
                  </>
                )}
              </Show>
            </div>

            <div class='sheet-actions'>
              <Show when={editable() && overrideSet()}>
                <button
                  type='button'
                  disabled={sending()}
                  onClick={() => void send('0')}
                >
                  Use the map's boxes
                </button>
              </Show>
              <button
                type='button'
                class='primary'
                disabled={!sendable() || sending()}
                onClick={apply}
              >
                Apply
              </button>
            </div>
          </aside>
        </div>
      </div>
    </div>
  )
}
