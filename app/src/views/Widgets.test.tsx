import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
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
    retention: 0.8,
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
      },
    ],
  }
}

function published(widgets: WidgetUsage[], audiences = ['all']): Usage {
  return {
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
})

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
          '30d': stats({ rank: 1, players: 50, retention: 0.2 }),
        }),
      }),
      widget({
        key: 'b',
        name: 'Bravo',
        windows: combined({
          '30d': stats({ rank: 2, players: 90, retention: 0.9 }),
        }),
      }),
      widget({
        key: 'c',
        name: 'Charlie',
        windows: combined({
          '30d': stats({ rank: 3, players: 70, retention: 0.5 }),
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

    fireEvent.click(getByRole('button', { name: /Kept on/ }))
    const sorted = [...container.querySelectorAll('thead th')].find(
      (th) => th.getAttribute('aria-sort') !== 'none',
    )
    expect(sorted?.textContent).toContain('Kept on')
    expect(sorted?.getAttribute('aria-sort')).toBe('descending')
    expect(names(container)).toEqual(['Bravo', 'Charlie', 'Alpha'])
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
    expect(line?.textContent).toContain('This version')
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
      /widget\/\d+x\d+\/widget:gui_ping_wheel/,
    )
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(container.querySelector('.widget-preview')).toBeNull()
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
