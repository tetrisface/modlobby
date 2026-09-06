import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Arrangement } from '../ipc/bindings/Arrangement'
import type { UserView } from '../ipc/bindings/UserView'
import { emptyLobby, setLobby } from '../store/lobby'
import { RoomProvider } from '../views/room/model'
import { onlineRoom } from '../views/room/online'
import { MapEditor } from './MapEditor'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

const OVERRIDE = 'game/modoptions/mapmetadata_startbox_override'

/** Lets awaited answers reach the component and the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function user(name: string, player: boolean): UserView {
  return {
    name,
    country: 'SE',
    userId: 1,
    lobbyClient: 'modlobby',
    status: {
      inGame: false,
      away: false,
      rank: 1,
      moderator: false,
      bot: false,
    },
    battleStatus: {
      ready: false,
      team: 0,
      allyTeam: 0,
      player,
      handicap: 0,
      sync: 'synced',
      side: 0,
    },
    battleId: 7,
  }
}

const twoBoxes: Arrangement = {
  startboxes: [
    {
      poly: [
        { x: 20, y: 20, strength: null },
        { x: 60, y: 60, strength: null },
      ],
    },
    {
      poly: [
        { x: 140, y: 140, strength: null },
        { x: 180, y: 180, strength: null },
      ],
    },
  ],
}

/** Rust, as far as the editor can tell: a room on `twoBoxes`, encoding by hand. */
function rustAnswers(arrangement: Arrangement | null) {
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    switch (command) {
      case 'current_arrangement':
        return arrangement ? { arrangement, source: 'override' } : null
      case 'decode_boxes':
        return null
      case 'encode_boxes': {
        const sent = (args as { arrangement: Arrangement }).arrangement
        return { value: 'x'.repeat(sent.startboxes.length * 40), limit: 989 }
      }
      case 'set_option':
        return undefined
      default:
        throw new Error(`unexpected ${command}`)
    }
  })
}

function room(me: string, player: boolean) {
  setLobby(emptyLobby())
  setLobby('me', me)
  setLobby('users', { [me]: user(me, player) })
  setLobby('myBattle', {
    boss: null,
    autoBalance: 'off',
    id: 7,
    gameHash: '',
    scriptTags: { [OVERRIDE]: 'blob' },
    vote: null,
    history: [],
  })
}

/** The drawing is 200 CSS pixels square here, so map units are pixels. */
function square(svg: SVGSVGElement) {
  svg.getBoundingClientRect = () =>
    ({
      left: 0,
      top: 0,
      width: 200,
      height: 200,
      right: 200,
      bottom: 200,
    }) as DOMRect
}

/**
 * The editor over the room the helper above sets up.
 *
 * It reads the room through `useRoom()` now, so a test has to say which room
 * it is editing -- here the online one, over the same mirrored state these
 * tests were already writing.
 */
function editor(onClose: () => void) {
  return (
    <RoomProvider value={onlineRoom()}>
      <MapEditor mapName='Comet Catcher' teams={2} onClose={onClose} />
    </RoomProvider>
  )
}

async function open(player = true) {
  room('me', player)
  const onClose = vi.fn()
  const result = render(() => editor(onClose))
  await settle()
  const svg = result.container.querySelector('svg') as SVGSVGElement
  square(svg)
  return { ...result, svg, onClose }
}

const drag = (
  svg: SVGSVGElement,
  from: [number, number],
  to: [number, number],
) => {
  fireEvent.pointerDown(svg, {
    button: 0,
    clientX: from[0],
    clientY: from[1],
    pointerId: 1,
  })
  fireEvent.pointerMove(svg, { clientX: to[0], clientY: to[1], pointerId: 1 })
  fireEvent.pointerUp(svg, { clientX: to[0], clientY: to[1], pointerId: 1 })
}

const boxes = (container: HTMLElement) => [
  ...container.querySelectorAll('.ed-box path'),
]
const button = (container: HTMLElement, label: string) =>
  [...container.querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === label,
  )!

beforeEach(() => rustAnswers(twoBoxes))
afterEach(cleanup)

describe('MapEditor', () => {
  test('starts from the boxes the room has and lists them', async () => {
    const { container } = await open()
    expect(boxes(container)).toHaveLength(2)
    expect(container.querySelectorAll('.ed-list li')).toHaveLength(2)
    expect(container.querySelector('.ed-list li .muted')?.textContent).toBe(
      'rectangle',
    )
  })

  test('a box dragged past the border stops there, and Apply sends the whole draft once', async () => {
    const { container, svg, onClose } = await open()
    drag(svg, [40, 40], [400, 40])
    await settle()
    // The first box's outline now hugs the right edge: x from 160 to 200.
    expect(boxes(container)[0]?.getAttribute('d')).toBe(
      'M160 20 L200 20 L200 60 L160 60 Z',
    )

    await new Promise((done) => setTimeout(done, 200))
    await settle()
    expect(container.querySelector('.ed-budget .mono')?.textContent).toBe(
      '80 / 989',
    )

    fireEvent.click(button(container, 'Apply'))
    await settle()
    const sent = vi
      .mocked(invoke)
      .mock.calls.filter(([command]) => command === 'set_option')
    expect(sent).toHaveLength(1)
    expect(sent[0]?.[1]).toEqual({
      key: 'mapmetadata_startbox_override',
      value: 'x'.repeat(80),
    })
    // Sent is done: the room's own minimap is where the echo shows up.
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  test('a drag released off the drawing still ends', async () => {
    const { container, svg } = await open()
    fireEvent.pointerDown(svg, {
      button: 0,
      clientX: 40,
      clientY: 40,
      pointerId: 1,
    })
    fireEvent.pointerMove(svg, { clientX: 90, clientY: 40, pointerId: 1 })
    // The pointer is captured, so the release arrives on the svg wherever it happened.
    fireEvent.pointerUp(svg, { clientX: 900, clientY: -50, pointerId: 1 })
    expect(boxes(container)[0]?.getAttribute('d')).toBe(
      'M160 0 L200 0 L200 40 L160 40 Z',
    )
    expect(button(container, 'Undo').disabled).toBe(false)
  })

  test('Escape cancels a drag in flight, and asks before closing a dirty draft', async () => {
    const { container, svg, onClose } = await open()
    const card = container.querySelector('.map-editor') as HTMLElement
    fireEvent.pointerDown(svg, {
      button: 0,
      clientX: 40,
      clientY: 40,
      pointerId: 1,
    })
    fireEvent.pointerMove(svg, { clientX: 90, clientY: 40, pointerId: 1 })
    fireEvent.keyDown(card, { key: 'Escape' })
    expect(boxes(container)[0]?.getAttribute('d')).toBe(
      'M20 20 L60 20 L60 60 L20 60 Z',
    )

    fireEvent.click(button(container, 'Add box'))
    fireEvent.keyDown(card, { key: 'Escape' })
    expect(onClose).not.toHaveBeenCalled()
    expect(container.querySelector('.ed-banner.warn')).not.toBeNull()
    fireEvent.click(button(container, 'Discard'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  test('the rectangle tool draws with a drag; Delete removes the selection', async () => {
    const { container, svg } = await open()
    fireEvent.click(button(container, 'Rectangle'))
    drag(svg, [100, 20], [130, 50])
    expect(boxes(container)).toHaveLength(3)
    expect(boxes(container)[2]?.getAttribute('d')).toBe(
      'M100 20 L130 20 L130 50 L100 50 Z',
    )
    fireEvent.keyDown(container.querySelector('.map-editor')!, {
      key: 'Delete',
    })
    expect(boxes(container)).toHaveLength(2)
  })

  test('the polygon tool places corners and closes on the first one', async () => {
    const { container, svg } = await open()
    fireEvent.click(button(container, 'Polygon'))
    for (const [x, y] of [
      [100, 100],
      [190, 100],
      [150, 190],
      [101, 101],
    ]) {
      fireEvent.pointerDown(svg, {
        button: 0,
        clientX: x,
        clientY: y,
        pointerId: 1,
      })
      fireEvent.pointerUp(svg, { clientX: x, clientY: y, pointerId: 1 })
    }
    expect(boxes(container)).toHaveLength(3)
    expect(boxes(container)[2]?.getAttribute('d')).toBe(
      'M100 100 L190 100 L150 190 Z',
    )
    expect(
      container.querySelector('.ed-list li:last-child .muted')?.textContent,
    ).toBe('3 corners')
  })

  test('a spectator sees the map and nothing to press', async () => {
    const { container, svg } = await open(false)
    expect(container.querySelector('.note')?.textContent).toBe(
      'read-only · spectator',
    )
    expect(button(container, 'Add box').disabled).toBe(true)
    expect(button(container, 'Apply').disabled).toBe(true)
    drag(svg, [40, 40], [400, 40])
    expect(boxes(container)[0]?.getAttribute('d')).toBe(
      'M20 20 L60 20 L60 60 L20 60 Z',
    )
  })

  test('the room moving under a dirty draft is announced, not applied', async () => {
    const { container } = await open()
    fireEvent.click(button(container, 'Add box'))
    setLobby('myBattle', 'history', [
      {
        seq: 1,
        key: 'mapmetadata_startbox_override',
        from: 'blob',
        to: 'other',
        by: 'Bob',
      },
    ])
    setLobby('myBattle', 'scriptTags', { [OVERRIDE]: 'other' })
    await settle()
    expect(boxes(container)).toHaveLength(3)
    expect(container.querySelector('.ed-banner')?.textContent).toContain(
      'changed by Bob',
    )
    fireEvent.click(button(container, 'Reload'))
    await settle()
    expect(boxes(container)).toHaveLength(2)
    expect(container.querySelector('.ed-banner')).toBeNull()
  })

  test('a number and a handle keep their shape on a map that is not square', async () => {
    // 400x200: the 0-200 space is stretched twice as wide as it is tall, which
    // is what puts the boxes on the map and what would squash everything else.
    const wide = {
      left: 0,
      top: 0,
      width: 400,
      height: 200,
      right: 400,
      bottom: 200,
    } as DOMRect
    const held = Element.prototype.getBoundingClientRect
    Element.prototype.getBoundingClientRect = () => wide
    try {
      room('me', true)
      const { container } = render(() => editor(vi.fn()))
      await settle()
      expect(
        container.querySelector('.ed-box text')?.getAttribute('transform'),
      ).toBe('translate(40 40) scale(0.5 1)')

      // The same anchor under a corner handle, so it is a circle and not an egg.
      fireEvent.pointerDown(container.querySelector('svg')!, {
        button: 0,
        clientX: 80,
        clientY: 40,
        pointerId: 1,
      })
      fireEvent.pointerUp(container.querySelector('svg')!, {
        clientX: 80,
        clientY: 40,
        pointerId: 1,
      })
      expect(
        container
          .querySelector('.ed-handles circle')
          ?.getAttribute('transform'),
      ).toBe('translate(20 20) scale(0.5 1)')
    } finally {
      Element.prototype.getBoundingClientRect = held
    }
  })

  test('a number and a handle grow with the map view', async () => {
    /** The first box's number and corner radius, with the drawing this big. */
    async function drawn(width: number, height: number) {
      const held = Element.prototype.getBoundingClientRect
      Element.prototype.getBoundingClientRect = () =>
        ({
          left: 0,
          top: 0,
          width,
          height,
          right: width,
          bottom: height,
        }) as DOMRect
      try {
        room('me', true)
        const { container, unmount } = render(() => editor(vi.fn()))
        await settle()
        // A press inside box 1, in map units either way round, so its handles
        // are drawn as well as its number.
        const svg = container.querySelector('svg')!
        const at = { clientX: width * 0.2, clientY: height * 0.2, pointerId: 1 }
        fireEvent.pointerDown(svg, { button: 0, ...at })
        fireEvent.pointerUp(svg, at)
        const size = {
          label: Number.parseFloat(
            (container.querySelector('.ed-box text') as SVGTextElement).style
              .fontSize,
          ),
          handle: Number(
            container.querySelector('.ed-handles circle')?.getAttribute('r'),
          ),
        }
        unmount()
        return size
      } finally {
        Element.prototype.getBoundingClientRect = held
      }
    }

    const small = await drawn(250, 250)
    const large = await drawn(900, 900)
    expect(large.label).toBeGreaterThan(small.label)
    expect(large.handle).toBeGreaterThan(small.handle)
  })

  test('fewer boxes than teams is said out loud', async () => {
    const { container } = await open()
    fireEvent.click(container.querySelector('.ed-list li .ed-pick')!)
    fireEvent.click(button(container, 'Delete'))
    expect(container.querySelector('.ed-warn')?.textContent).toContain(
      '1 team has no box',
    )
  })
})
