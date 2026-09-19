import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Fork } from '../ipc/bindings/Fork'
import type { Install } from '../ipc/bindings/Install'
import type { InstalledWidget } from '../ipc/bindings/InstalledWidget'
import type { LocalWidget } from '../ipc/bindings/LocalWidget'
import type { Usage } from '../ipc/bindings/Usage'
import type { WidgetStatus } from '../ipc/bindings/WidgetStatus'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'

vi.mock('@tauri-apps/api/core', async () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string, protocol: string) =>
    `http://${protocol}.localhost/${encodeURIComponent(path)}`,
}))

const asked = vi.mocked(invoke)

function stats(over: Partial<WindowStats> = {}): WindowStats {
  return {
    rank: 1,
    players: 100,
    players_active: 80,
    players_still_using: 70,
    retention: 0.8,
    still_using: 0.7,
    withheld: false,
    sightings: 300,
    replays: 200,
    days_covered: 30,
    coverage: 1,
    ...over,
  }
}

/** Windows nest under an audience; `all` is what most tests mean. */
function combined(windows: Record<string, WindowStats>) {
  return { all: windows }
}

function widget(over: Partial<WidgetUsage> = {}): WidgetUsage {
  return {
    key: 'widget:gui_ping_wheel',
    resolved: true,
    id: 'gui_ping_wheel',
    name: 'Ping Wheel',
    author: 'Errrrrrr',
    description: 'A radial ping menu',
    windows: combined({ '30d': stats() }),
    install: noSource(),
    image: '',
    images: [],
    first_published: '',
    last_updated: '',
    main: '',
    forks: [],
    ...over,
  }
}

function noSource(): Install {
  return {
    kind: 'none',
    url: '',
    archive: false,
    page: '',
    license: '',
    permissive: false,
    reason: 'no known source',
    files: [],
  }
}

function fromGithub(): Install {
  return {
    kind: 'github',
    url: 'https://raw.githubusercontent.com/o/r/c6fd104/gui.lua',
    archive: false,
    page: 'https://github.com/o/r',
    license: 'MIT',
    permissive: true,
    reason: '',
    files: [
      {
        path: 'gui.lua',
        content_hash: 'aGFzaA==',
        url: 'https://raw.githubusercontent.com/o/r/c6fd104/gui.lua',
        install_path: 'gui.lua',
      },
    ],
  }
}

function published(widgets: WidgetUsage[], audiences = ['all']): Usage {
  return {
    document_version: 4,
    generated_at: '2026-09-16T04:00:00+00:00',
    policy_version: 'pve_widget_harvest_v2_prefix_262144_audience',
    audiences,
    windows: ['7d', '30d', '90d', '365d', 'all'],
    widgets,
  }
}

function installedStatus(over: Partial<WidgetStatus> = {}): WidgetStatus {
  return {
    installed: [],
    configured: [],
    locked: false,
    writeDir: '/home/someone/.local/share/modlobby/data',
    local: [],
    ...over,
  }
}

function serve(usage: Usage | null, installed = installedStatus()) {
  asked.mockImplementation(async (command: string) => {
    if (command === 'widget_usage') return usage
    if (command === 'widget_installed') return installed
    throw new Error(`unexpected ${command}`)
  })
}

/**
 * The store behind the page holds the document for the run, so the view is
 * imported per test rather than once: a module kept between them would answer
 * the next test with the last one's widgets.
 */
async function fresh() {
  vi.resetModules()
  return (await import('./Widgets')).Widgets
}

const names = (container: HTMLElement) =>
  [...container.querySelectorAll('.widget-name strong')].map(
    (name) => name.textContent ?? '',
  )

const rows = (container: HTMLElement) =>
  container.querySelectorAll('.widget-table tbody tr')

const buttons = (container: HTMLElement) =>
  [...container.querySelectorAll('.widget-buttons button')].map(
    (button) => button.textContent ?? '',
  )

async function drawn(container: HTMLElement) {
  await waitFor(() => expect(rows(container).length).toBeGreaterThan(0))
}

beforeEach(() => {
  // A block body, not an expression: a value returned from `beforeEach` that
  // happens to be callable is taken for a teardown hook and called with the
  // test context — which for a mock means one phantom invoke per test.
  asked.mockReset()
  // The toolbar remembers itself across mounts, so each test gets an empty
  // store of its own -- otherwise a filter one test clicks is a default the
  // next one starts from. happy-dom ships no `localStorage`, which is also why
  // the page has to work without one.
  Object.defineProperty(window, 'localStorage', {
    value: memoryStore(),
    configurable: true,
  })
})

/** Enough of `Storage` for a page that only gets, sets and clears. */
function memoryStore(): Storage {
  const held = new Map<string, string>()
  return {
    getItem: (key: string) => held.get(key) ?? null,
    setItem: (key: string, value: string) => void held.set(key, value),
    removeItem: (key: string) => void held.delete(key),
    clear: () => held.clear(),
    key: (at: number) => [...held.keys()][at] ?? null,
    get length() {
      return held.size
    },
  } as Storage
}

afterEach(cleanup)

describe('the widgets page', () => {
  test('widgets are listed in the window rank, most used first', async () => {
    serve(
      published([
        widget({
          key: 'b',
          name: 'Second',
          windows: combined({ '30d': stats({ rank: 2 }) }),
        }),
        widget({
          key: 'a',
          name: 'First',
          windows: combined({ '30d': stats({ rank: 1 }) }),
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(names(container)).toEqual(['First', 'Second'])
    // The place shown is the position on the page, so the list reads 1, 2
    // whichever window is on.
    expect(
      [...container.querySelectorAll('tbody td.num:first-child')].map(
        (at) => at.textContent,
      ),
    ).toEqual(['1', '2'])
  })

  test('the numbers shown are the ones for the window picked', async () => {
    serve(
      published([
        widget({
          name: 'Ping Wheel',
          windows: combined({
            '30d': stats({ rank: 1, players: 120 }),
            all: stats({ rank: 1, players: 400 }),
          }),
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container, getByText } = render(() => <Widgets />)

    await drawn(container)
    const players = () =>
      container.querySelector('tbody td.num:nth-child(3)')?.textContent
    expect(players()).toBe('120')
    fireEvent.click(getByText('All time'))
    expect(players()).toBe('400')
  })

  test('a widget the window withheld is not listed in it', async () => {
    // The anonymity floor is applied inside each window, so a widget with
    // numbers for the year and none for the week is absent from the week
    // rather than shown there with a small number.
    serve(
      published([
        widget({
          key: 'a',
          name: 'Everyday',
          windows: combined({ '30d': stats(), all: stats() }),
        }),
        widget({
          key: 'b',
          name: 'Rare',
          windows: combined({ all: stats({ rank: 2 }) }),
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container, getByText } = render(() => <Widgets />)

    await drawn(container)
    expect(names(container)).toEqual(['Everyday'])
    fireEvent.click(getByText('All time'))
    expect(names(container)).toEqual(['Everyday', 'Rare'])
  })

  test('a window that has only been partly harvested says so', async () => {
    serve(
      published([
        widget({
          windows: combined({
            '30d': stats({ days_covered: 2, coverage: 0.06 }),
          }),
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(container.querySelector('.chip.warn')?.textContent).toBe(
      '2 days harvested',
    )
  })

  test('a widget nothing could be resolved for is still named', async () => {
    serve(
      published([
        widget({
          key: 'unresolved:abc',
          resolved: false,
          id: '',
          name: 'Flea Transport',
          author: '[teh]Teddy',
          description: '',
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(names(container)).toEqual(['Flea Transport'])
    // Nothing to point an install at, so no hub mark.
    expect(container.querySelector('.chip.ok')).toBeNull()
    // And no button that would do nothing: it says why instead.
    expect(buttons(container)).toEqual([])
    expect(container.querySelector('.widget-why')?.textContent).toBe(
      'no known source',
    )
  })

  test('a widget with a download offers to install it', async () => {
    serve(published([widget({ install: fromGithub() })]))
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(buttons(container)).toEqual(['Install'])
  })

  test('an installed widget offers to switch it on and to remove it, not to install it again', async () => {
    const entry: InstalledWidget = {
      key: 'widget:gui_ping_wheel',
      name: 'Ping Wheel',
      files: ['LuaUI/Widgets/gui.lua'],
      hashes: ['aGFzaA=='],
      source: 'https://example.test/gui.lua',
      installed_at: 1n,
      settings_before: [],
    }
    serve(
      published([widget({ install: fromGithub() })]),
      installedStatus({ installed: [entry] }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    // BAR starts a new user widget switched off, and has no entry for it yet.
    expect(buttons(container)).toEqual(['Enable', 'Delete'])
  })

  test('an installed widget whose published files moved on offers an update', async () => {
    const entry: InstalledWidget = {
      key: 'widget:gui_ping_wheel',
      name: 'Ping Wheel',
      files: ['LuaUI/Widgets/gui.lua'],
      hashes: ['an older revision'],
      source: 'https://example.test/gui.lua',
      installed_at: 1n,
      settings_before: [],
    }
    serve(
      published([widget({ install: fromGithub() })]),
      installedStatus({ installed: [entry] }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(buttons(container)).toEqual(['Update', 'Enable', 'Delete'])
  })

  test('a widget BAR knows about can be switched off from here', async () => {
    serve(
      published([widget()]),
      installedStatus({
        configured: [{ name: 'Ping Wheel', order: 5n, has_settings: true }],
      }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(buttons(container)).toEqual(['Disable'])
  })

  test('a widget already switched off offers to switch it back on', async () => {
    serve(
      published([widget()]),
      installedStatus({
        configured: [{ name: 'Ping Wheel', order: 0n, has_settings: true }],
      }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    expect(buttons(container)).toEqual(['Enable'])
  })

  test('while a game is running the config buttons are held', async () => {
    // BAR rewrites its widget config wholesale on exit, so an edit made now
    // would be discarded without a word.
    serve(
      published([widget()]),
      installedStatus({
        locked: true,
        configured: [{ name: 'Ping Wheel', order: 5n, has_settings: true }],
      }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await drawn(container)
    const disable = container.querySelector(
      '.widget-buttons button',
    ) as HTMLButtonElement
    expect(disable.disabled).toBe(true)
    expect(disable.title).toContain('game is running')
  })

  test('search narrows the list to what the reader asked for', async () => {
    serve(
      published([
        widget({ key: 'a', name: 'Ping Wheel', author: 'Errrrrrr' }),
        widget({
          key: 'b',
          name: 'Raptor Grid',
          author: 'Lu5ck',
          windows: combined({ '30d': stats({ rank: 2 }) }),
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container, getByPlaceholderText } = render(() => <Widgets />)

    await drawn(container)
    expect(names(container)).toEqual(['Ping Wheel', 'Raptor Grid'])
    fireEvent.input(getByPlaceholderText(/Search/), {
      target: { value: 'lu5ck' },
    })
    expect(names(container)).toEqual(['Raptor Grid'])
  })

  test('the pve and pvp filter appears only when the split is published', async () => {
    serve(published([widget()]))
    const Widgets = await fresh()
    const { container, queryByText } = render(() => <Widgets />)

    await drawn(container)
    // A lone "All" button is a control that does nothing.
    expect(queryByText('PvE')).toBeNull()
  })

  test('picking pve shows only what was seen against ai', async () => {
    serve(
      published(
        [
          widget({
            key: 'a',
            name: 'Raptor Grid',
            windows: {
              all: { '30d': stats({ rank: 1 }) },
              pve: { '30d': stats({ rank: 1 }) },
            },
          }),
          widget({
            key: 'b',
            name: 'Build Order',
            windows: {
              all: { '30d': stats({ rank: 2 }) },
              pvp: { '30d': stats({ rank: 1 }) },
            },
          }),
        ],
        ['all', 'pve', 'pvp'],
      ),
    )
    const Widgets = await fresh()
    const { container, getByText } = render(() => <Widgets />)

    await drawn(container)
    expect(names(container)).toEqual(['Raptor Grid', 'Build Order'])
    fireEvent.click(getByText('PvE'))
    expect(names(container)).toEqual(['Raptor Grid'])
    fireEvent.click(getByText('PvP'))
    expect(names(container)).toEqual(['Build Order'])
  })

  test('a document that could not be fetched leaves a page that still reads', async () => {
    serve(null)
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await waitFor(() =>
      expect(container.querySelector('p.muted')?.textContent).toContain(
        'could not be fetched',
      ),
    )
    expect(rows(container).length).toBe(0)
  })
})

function onDisk(over: Partial<LocalWidget> = {}): LocalWidget {
  return {
    name: 'Ping Wheel',
    file: 'LuaUI/Widgets/gui_ping_wheel.lua',
    dir: 'C:/Users/someone/AppData/Local/Programs/Beyond-All-Reason/data',
    writable: false,
    hash: 'aGFzaA==',
    hash_text: 'aGFzaA==',
    ...over,
  }
}

const cell = (container: HTMLElement, row: number, column: number) =>
  rows(container)[row]?.querySelector(`td:nth-child(${column})`)?.textContent

describe('sorting', () => {
  const three = () =>
    published([
      widget({
        key: 'a',
        name: 'Alpha',
        windows: combined({
          '30d': stats({
            rank: 1,
            players: 50,
            retention: 0.2,
            still_using: 0.2,
          }),
        }),
      }),
      widget({
        key: 'b',
        name: 'Bravo',
        windows: combined({
          '30d': stats({
            rank: 2,
            players: 90,
            retention: 0.9,
            still_using: 0.9,
          }),
        }),
      }),
      widget({
        key: 'c',
        name: 'Charlie',
        windows: combined({
          '30d': stats({
            rank: 3,
            players: 70,
            retention: 0.5,
            still_using: 0.5,
          }),
        }),
      }),
    ])

  test('every header sorts', async () => {
    serve(three())
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    const headers = [...container.querySelectorAll('thead th')]
    expect(headers.length).toBeGreaterThan(5)
    expect(headers.every((th) => th.querySelector('button.sort-head'))).toBe(
      true,
    )
  })

  test('a number column sorts most first, and a second click flips it', async () => {
    serve(three())
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)

    fireEvent.click(getByRole('button', { name: /Players/ }))
    expect(names(container)).toEqual(['Bravo', 'Charlie', 'Alpha'])
    fireEvent.click(getByRole('button', { name: /Players/ }))
    expect(names(container)).toEqual(['Alpha', 'Charlie', 'Bravo'])
  })

  test('the sorted header says which way it sorts', async () => {
    serve(three())
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)

    fireEvent.click(getByRole('button', { name: /Still using/ }))
    const sorted = [...container.querySelectorAll('thead th')].find(
      (th) => th.getAttribute('aria-sort') !== 'none',
    )
    expect(sorted?.textContent).toContain('Still using')
    expect(sorted?.getAttribute('aria-sort')).toBe('descending')
    expect(names(container)).toEqual(['Bravo', 'Charlie', 'Alpha'])
  })

  test('the sort and the toolbar are still set on the next visit', async () => {
    serve(three())
    const Widgets = await fresh()
    const first = render(() => <Widgets />)
    await drawn(first.container)
    fireEvent.click(first.getByRole('button', { name: /Players/ }))
    fireEvent.click(first.getByRole('button', { name: 'Include used once' }))
    cleanup()

    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)
    expect(names(container)).toEqual(['Bravo', 'Charlie', 'Alpha'])
    expect(
      getByRole('button', { name: 'Include used once' }).getAttribute(
        'aria-pressed',
      ),
    ).toBe('true')
  })

  test('a text column sorts A to Z first', async () => {
    serve(three())
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByRole('button', { name: /Players/ }))
    fireEvent.click(getByRole('button', { name: /^Widget/ }))
    expect(names(container)).toEqual(['Alpha', 'Bravo', 'Charlie'])
  })
})

describe('the installed filter', () => {
  test('shows only widgets with a file on this machine', async () => {
    serve(
      published([
        widget({ key: 'a', name: 'Ping Wheel' }),
        widget({
          key: 'b',
          name: 'Raptor Grid',
          windows: combined({ '30d': stats({ rank: 2 }) }),
        }),
      ]),
      installedStatus({ local: [onDisk()] }),
    )
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)

    expect(container.querySelector('.count')?.textContent).toContain(
      '1 installed',
    )
    fireEvent.click(getByRole('button', { name: 'Installed' }))
    expect(names(container)).toEqual(['Ping Wheel'])
  })

  test('a name in BAR config alone does not count as installed', async () => {
    // The config keeps entries for widgets deleted long ago.
    serve(
      published([widget()]),
      installedStatus({
        configured: [{ name: 'Ping Wheel', order: 5n, has_settings: true }],
      }),
    )
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByRole('button', { name: 'Installed' }))
    expect(rows(container).length).toBe(0)
    expect(container.querySelector('p.muted')?.textContent).toContain(
      'None of the widgets',
    )
  })
})

describe('what is on this machine', () => {
  test('a file that is the published revision says so', async () => {
    serve(
      published([widget({ install: fromGithub() })]),
      installedStatus({ local: [onDisk({ hash: 'aGFzaA==' })] }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    const line = container.querySelector('.widget-local li')
    expect(line?.classList.contains('exact')).toBe(true)
    expect(line?.textContent).toContain('Main version')
    expect(line?.textContent).toContain('gui_ping_wheel.lua')
    expect(line?.textContent).toContain("BAR's folder")
  })

  test('a homebrewed file under the same name is told apart', async () => {
    // The case that prompted this: a local "Dont Stand in Fire" matching no
    // published revision, which BAR's config cannot distinguish.
    serve(
      published([widget({ install: fromGithub() })]),
      installedStatus({
        local: [onDisk({ hash: 'bWluZQ==', hash_text: 'bWluZQ==' })],
      }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    const line = container.querySelector('.widget-local li')
    expect(line?.classList.contains('exact')).toBe(false)
    expect(line?.textContent).toContain('Your own version')
  })

  test('two files with one name are explained', async () => {
    serve(
      published([widget()]),
      installedStatus({
        local: [
          onDisk(),
          onDisk({
            file: 'LuaUI/Widgets/copy/gui_ping_wheel.lua',
            hash: 'b3RoZXI=',
          }),
        ],
      }),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    expect(container.querySelector('.widget-local')?.textContent).toContain(
      '2 files declare this name',
    )
  })
})

describe('dates', () => {
  test('when it was last updated and first published read as ages, exact on hover', async () => {
    const updated = new Date(Date.now() - 400 * 86_400_000).toISOString()
    serve(published([widget({ last_updated: updated, first_published: '' })]))
    const Widgets = await fresh()
    const { container, getByText } = render(() => <Widgets />)
    await drawn(container)
    const cell = getByText('1y 1m ago')
    expect(cell.getAttribute('title')).toBe(new Date(updated).toLocaleString())
    expect(container.querySelector('th')?.parentElement?.textContent).toContain(
      'Updated',
    )
    expect(container.querySelector('th')?.parentElement?.textContent).toContain(
      'Published',
    )
  })

  test('Updated sorts newest first on the first click, and undated last', async () => {
    const at = (days: number) =>
      new Date(Date.now() - days * 86_400_000).toISOString()
    serve(
      published([
        widget({ key: 'a', name: 'Old', last_updated: at(300) }),
        widget({ key: 'b', name: 'Undated', last_updated: '' }),
        widget({ key: 'c', name: 'Fresh', last_updated: at(2) }),
      ]),
    )
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByRole('button', { name: /Updated/ }))
    expect(names(container)).toEqual(['Fresh', 'Old', 'Undated'])
  })
})

describe('pictures', () => {
  test('a widget with a picture asks the thumbnail scheme for it by key', async () => {
    serve(
      published([widget({ image: 'https://widget-hub.example/cover.png' })]),
    )
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    const img = container.querySelector('.widget-thumb img') as HTMLImageElement
    // A key, never the URL: the scheme only serves what the document names.
    expect(decodeURIComponent(img.src)).toContain('widget:gui_ping_wheel')
    expect(decodeURIComponent(img.src)).not.toContain('widget-hub.example')
  })

  test('the picture opens about four rows tall and closes again', async () => {
    serve(
      published([widget({ image: 'https://widget-hub.example/cover.png' })]),
    )
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)

    fireEvent.click(getByLabelText(/Show a larger picture/))
    const preview = container.querySelector(
      '.widget-preview img',
    ) as HTMLImageElement
    expect(preview).not.toBeNull()
    expect(decodeURIComponent(preview.src)).toMatch(
      /widget\/\d+x\d+\/0\/widget:gui_ping_wheel/,
    )
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(container.querySelector('.widget-preview')).toBeNull()
  })

  test('a press anywhere outside an open picture closes it', async () => {
    serve(
      published([widget({ image: 'https://widget-hub.example/cover.png' })]),
    )
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)

    fireEvent.click(getByLabelText(/Show a larger picture/))
    expect(container.querySelector('.widget-preview')).not.toBeNull()
    // Inside the picture's own corner of the row, it stays open.
    fireEvent.pointerDown(
      container.querySelector('.widget-preview') as HTMLElement,
    )
    expect(container.querySelector('.widget-preview')).not.toBeNull()

    fireEvent.pointerDown(
      container.querySelector('.widget-text') as HTMLElement,
    )
    expect(container.querySelector('.widget-preview')).toBeNull()
  })

  const FIRST = 'https://cdn.example/1.webp'
  const GALLERY = [1, 2, 3, 4, 5, 6, 7].map(
    (n) => `https://cdn.example/${n}.webp`,
  )
  const gallery = () => published([widget({ image: FIRST, images: GALLERY })])
  const pictureIndex = (img: Element | null) =>
    decodeURIComponent((img as HTMLImageElement).src).match(
      /widget\/\d+x\d+\/(\d+)\//,
    )?.[1]

  test('hovering the tile shows the other pictures beside it, and leaving hides them', async () => {
    serve(gallery())
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    expect(container.querySelector('.widget-companions')).toBeNull()

    fireEvent.pointerEnter(getByLabelText(/Show a larger picture/))
    const companions = container.querySelectorAll('.widget-companions button')
    expect(companions).toHaveLength(4)
    expect(pictureIndex(companions.item(0).querySelector('img'))).toBe('1')
    // Seven pictures, four beside the tile: the last counts the other two.
    expect(container.querySelector('.widget-more')?.textContent).toBe('+2')

    fireEvent.pointerLeave(
      container.querySelector('.widget-thumb-anchor') as HTMLElement,
    )
    expect(container.querySelector('.widget-companions')).toBeNull()
  })

  test('a companion opens the gallery at its own picture', async () => {
    serve(gallery())
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.pointerEnter(getByLabelText(/Show a larger picture/))
    fireEvent.click(getByLabelText(/Show picture 3 of 7/))
    expect(
      pictureIndex(container.querySelector('.widget-preview-picture img')),
    ).toBe('2')
    expect(
      container
        .querySelector('.widget-film [aria-current="true"]')
        ?.getAttribute('aria-label'),
    ).toBe('Picture 3 of 7')
  })

  test('the strip switches pictures and the arrows step through them, wrapping', async () => {
    serve(gallery())
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByLabelText(/Show a larger picture/))
    const shown = () =>
      pictureIndex(container.querySelector('.widget-preview-picture img'))
    expect(shown()).toBe('0')

    fireEvent.click(getByLabelText('Picture 5 of 7'))
    expect(shown()).toBe('4')
    fireEvent.keyDown(window, { key: 'ArrowRight' })
    expect(shown()).toBe('5')
    fireEvent.keyDown(window, { key: 'ArrowRight' })
    fireEvent.keyDown(window, { key: 'ArrowRight' })
    expect(shown()).toBe('0')
    fireEvent.keyDown(window, { key: 'ArrowLeft' })
    expect(shown()).toBe('6')
  })

  test('a widget with one picture has no companions and no strip', async () => {
    serve(published([widget({ image: FIRST, images: [FIRST] })]))
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.pointerEnter(getByLabelText(/Show a larger picture/))
    expect(container.querySelector('.widget-companions')).toBeNull()
    fireEvent.click(getByLabelText(/Show a larger picture/))
    expect(container.querySelector('.widget-preview')).not.toBeNull()
    expect(container.querySelector('.widget-film')).toBeNull()
  })

  test('a widget with no picture gets its initials rather than a broken image', async () => {
    serve(published([widget({ name: 'Dont Stand in Fire', image: '' })]))
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    expect(
      container.querySelector('.widget-thumb.placeholder')?.textContent,
    ).toBe('DS')
    expect(container.querySelector('.widget-thumb img')).toBeNull()
  })

  test('a picture that fails to load falls back to initials', async () => {
    serve(published([widget({ image: 'https://widget-hub.example/gone.png' })]))
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.error(
      container.querySelector('.widget-thumb img') as HTMLImageElement,
    )
    expect(container.querySelector('.widget-thumb.placeholder')).not.toBeNull()
  })
})

function fork(over: Partial<Fork> = {}): Fork {
  return {
    key: 'github:o/r:Ping Wheel',
    kind: 'lineage',
    main: false,
    id: 'gui_ping_wheel',
    author: 'Errrrrrr',
    description: '',
    install: fromGithub(),
    image: '',
    images: [],
    first_published: '',
    last_updated: '',
    windows: combined({ '30d': stats() }),
    ...over,
  }
}

/** A name published by two repositories, plus players nobody could trace. */
function family(): WidgetUsage {
  return widget({
    main: 'github:o/r:Ping Wheel',
    install: fromGithub(),
    forks: [
      fork({ main: true }),
      fork({
        key: 'github:bob/w:Ping Wheel',
        author: 'Bob',
        install: {
          ...fromGithub(),
          page: 'https://github.com/bob/w',
          files: [{ ...fromGithub().files[0]!, content_hash: 'Ym9i' }],
        },
        windows: combined({ '30d': stats({ players: 12 }) }),
      }),
      fork({
        key: 'other',
        kind: 'other',
        author: '',
        install: noSource(),
        windows: combined({
          '30d': stats({
            withheld: true,
            players: 0,
            players_active: 0,
            players_still_using: 0,
          }),
        }),
      }),
    ],
  })
}

const forkRows = (container: HTMLElement) => [
  ...container.querySelectorAll('tbody tr.widget-fork'),
]

describe('still using or used once', () => {
  test('the page counts "still using" until asked otherwise', async () => {
    serve(published([widget()]))
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)

    const toggle = getByRole('button', { name: 'Include used once' })
    expect(toggle.getAttribute('aria-pressed')).toBe('false')
    expect(container.querySelector('thead')?.textContent).toContain(
      'Still using',
    )
    // 70 of 100 still using: 70%, and 30 off.
    expect(cell(container, 0, 4)).toBe('70%')
    expect(cell(container, 0, 5)).toBe('30')
  })

  test('including used once switches the header, the share and the off count', async () => {
    serve(published([widget()]))
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)

    fireEvent.click(getByRole('button', { name: 'Include used once' }))
    expect(
      getByRole('button', { name: 'Include used once' }).getAttribute(
        'aria-pressed',
      ),
    ).toBe('true')
    const head = container.querySelector('thead')?.textContent ?? ''
    expect(head).toContain('Used once')
    expect(head).not.toContain('Still using')
    expect(cell(container, 0, 4)).toBe('80%')
    expect(cell(container, 0, 5)).toBe('20')
  })

  test('sorting follows the counting the page is showing', async () => {
    serve(
      published([
        widget({
          key: 'a',
          name: 'Tried',
          windows: combined({
            '30d': stats({ rank: 1, retention: 0.9, still_using: 0.1 }),
          }),
        }),
        widget({
          key: 'b',
          name: 'Kept',
          windows: combined({
            '30d': stats({ rank: 2, retention: 0.5, still_using: 0.5 }),
          }),
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container, getByRole } = render(() => <Widgets />)
    await drawn(container)

    fireEvent.click(getByRole('button', { name: /Still using/ }))
    expect(names(container)).toEqual(['Kept', 'Tried'])
    fireEvent.click(getByRole('button', { name: 'Include used once' }))
    expect(names(container)).toEqual(['Tried', 'Kept'])
  })
})

describe('versions under a name', () => {
  test('a row with several versions opens to them, one level deep', async () => {
    serve(published([family()]))
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)

    expect(forkRows(container)).toHaveLength(0)
    fireEvent.click(getByLabelText(/Show the 3 versions of Ping Wheel/))
    const kinds = forkRows(container).map(
      (row) => row.querySelector('.widget-fork-kind')?.textContent,
    )
    expect(kinds).toEqual(['Main version', 'Fork', 'Other versions'])
    // Nothing inside a version opens further.
    expect(
      container.querySelectorAll('tr.widget-fork .widget-expand'),
    ).toHaveLength(0)
  })

  test('clicking the row itself opens it, but clicking what it holds does not', async () => {
    serve(published([family()]))
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)

    const row = container.querySelector('tr.widget-row') as HTMLElement
    fireEvent.click(row.querySelector('.widget-text strong') as HTMLElement)
    expect(forkRows(container)).toHaveLength(3)

    // The chevron acts once, not twice: the row ignores clicks on a button.
    fireEvent.click(row.querySelector('.widget-expand') as HTMLElement)
    expect(forkRows(container)).toHaveLength(0)

    fireEvent.click(row.querySelector('.widget-text strong') as HTMLElement)
    expect(forkRows(container)).toHaveLength(3)
  })

  test('a row with one version has nothing to open', async () => {
    serve(published([widget()]))
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)
    expect(container.querySelector('.widget-expand')).toBeNull()
  })

  test('a withheld version reads "few", not zero', async () => {
    serve(published([family()]))
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByLabelText(/Show the 3 versions/))
    const other = forkRows(container)[2]!
    expect(other.querySelector('td.num')?.textContent).toBe('few')
  })

  test('switching on and off lives on the name, never on a version', async () => {
    // BAR has one config entry per name, so a second Disable on a version
    // would be a second button for the same switch.
    serve(
      published([family()]),
      installedStatus({
        configured: [{ name: 'Ping Wheel', order: 5n, has_settings: true }],
      }),
    )
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByLabelText(/Show the 3 versions/))

    const rowButtons = [
      ...container.querySelectorAll('tr.widget-row .widget-buttons button'),
    ].map((button) => button.textContent)
    expect(rowButtons).toContain('Disable')
    const versionButtons = forkRows(container).flatMap((row) =>
      [...row.querySelectorAll('.widget-buttons button')].map(
        (button) => button.textContent,
      ),
    )
    expect(versionButtons).not.toContain('Disable')
    expect(versionButtons).not.toContain('Enable')
    // Each published version installs on its own.
    expect(versionButtons.filter((label) => label === 'Install')).toHaveLength(
      2,
    )
  })

  test('a homebrew under the same name shows as your version', async () => {
    serve(
      published([family()]),
      installedStatus({
        local: [onDisk({ hash: 'bWluZQ==', hash_text: 'bWluZQ==' })],
      }),
    )
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByLabelText(/Show the 4 versions/))
    const yours = container.querySelector('tr.widget-fork.yours')
    expect(yours?.textContent).toContain('Your version')
    expect(yours?.textContent).toContain('gui_ping_wheel.lua')
    // Not modlobby's to remove.
    expect(yours?.querySelectorAll('button')).toHaveLength(0)
  })

  test('installing a version installs that version, by its own key', async () => {
    const calls: Array<{ command: string; args: unknown }> = []
    asked.mockImplementation(async (command: string, args?: unknown) => {
      calls.push({ command, args })
      if (command === 'widget_usage') return published([family()])
      if (command === 'widget_installed') return installedStatus()
      if (command === 'widget_install') return {}
      throw new Error(`unexpected ${command}`)
    })
    const Widgets = await fresh()
    const { container, getByLabelText } = render(() => <Widgets />)
    await drawn(container)
    fireEvent.click(getByLabelText(/Show the 3 versions/))

    const bob = forkRows(container)[1]!
    fireEvent.click(
      bob.querySelector('.widget-buttons button') as HTMLButtonElement,
    )
    await waitFor(() =>
      expect(calls.some((call) => call.command === 'widget_install')).toBe(
        true,
      ),
    )
    const install = calls.find((call) => call.command === 'widget_install')!
    expect((install.args as { key: string }).key).toBe(
      'github:bob/w:Ping Wheel',
    )
  })
})

describe('source links', () => {
  test('a source is a button that opens the system browser', async () => {
    const opened: string[] = []
    asked.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'widget_usage')
        return published([widget({ install: fromGithub() })])
      if (command === 'widget_installed') return installedStatus()
      if (command === 'open_url') {
        opened.push((args as { url: string }).url)
        return undefined
      }
      throw new Error(`unexpected ${command}`)
    })
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)
    await drawn(container)

    const button = container.querySelector(
      'tr.widget-row .link-button',
    ) as HTMLButtonElement
    expect(button.tagName).toBe('BUTTON')
    expect(button.textContent).toContain('GitHub')
    expect(button.querySelector('use')?.getAttribute('href')).toBe(
      '#act-external',
    )
    expect(container.querySelector('tr.widget-row a[href]')).toBeNull()
    fireEvent.click(button)
    await waitFor(() => expect(opened).toEqual(['https://github.com/o/r']))
  })
})
