import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Usage } from '../ipc/bindings/Usage'
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

function widget(over: Partial<WidgetUsage> = {}): WidgetUsage {
  return {
    key: 'widget:gui_ping_wheel',
    resolved: true,
    id: 'gui_ping_wheel',
    name: 'Ping Wheel',
    author: 'Errrrrrr',
    description: 'A radial ping menu',
    windows: { '30d': stats() },
    ...over,
  }
}

function published(widgets: WidgetUsage[]): Usage {
  return {
    generated_at: '2026-09-16T04:00:00+00:00',
    policy_version: 'pve_widget_harvest_v1',
    windows: ['7d', '30d', '90d', '365d', 'all'],
    widgets,
  }
}

function serve(usage: Usage | null) {
  asked.mockImplementation(async (command: string) => {
    if (command === 'widget_usage') return usage
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
  [...container.querySelectorAll('.widget-text h2')].map(
    (name) => name.textContent ?? '',
  )

const cards = (container: HTMLElement) =>
  container.querySelectorAll('.widget-card')

async function drawn(container: HTMLElement) {
  await waitFor(() => expect(cards(container).length).toBeGreaterThan(0))
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
          windows: { '30d': stats({ rank: 2 }) },
        }),
        widget({
          key: 'a',
          name: 'First',
          windows: { '30d': stats({ rank: 1 }) },
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
      [...container.querySelectorAll('.widget-rank')].map(
        (at) => at.textContent,
      ),
    ).toEqual(['1', '2'])
  })

  test('the numbers shown are the ones for the window picked', async () => {
    serve(
      published([
        widget({
          name: 'Ping Wheel',
          windows: {
            '30d': stats({ rank: 1, players: 120 }),
            all: stats({ rank: 1, players: 400 }),
          },
        }),
      ]),
    )
    const Widgets = await fresh()
    const { container, getByText } = render(() => <Widgets />)

    await drawn(container)
    expect(container.querySelector('.widget-stats dd')?.textContent).toBe('120')
    fireEvent.click(getByText('All time'))
    expect(container.querySelector('.widget-stats dd')?.textContent).toBe('400')
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
          windows: { '30d': stats(), all: stats() },
        }),
        widget({
          key: 'b',
          name: 'Rare',
          windows: { all: stats({ rank: 2 }) },
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
          windows: { '30d': stats({ days_covered: 2, coverage: 0.06 }) },
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
  })

  test('a document that could not be fetched leaves a page that still reads', async () => {
    serve(null)
    const Widgets = await fresh()
    const { container } = render(() => <Widgets />)

    await waitFor(() =>
      expect(container.querySelector('.muted')?.textContent).toContain(
        'could not be fetched',
      ),
    )
    expect(cards(container).length).toBe(0)
  })
})
