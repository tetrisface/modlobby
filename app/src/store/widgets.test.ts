import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, test, vi } from 'vitest'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import type { Usage } from '../ipc/bindings/Usage'
import {
  WINDOW_ORDER,
  disabledOnly,
  isRepresentative,
  statsFor,
} from './widgets'

vi.mock('@tauri-apps/api/core', async () => ({ invoke: vi.fn() }))

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

function widget(windows: Record<string, WindowStats>): WidgetUsage {
  return {
    key: 'widget:gui_ping_wheel',
    resolved: true,
    id: 'gui_ping_wheel',
    name: 'Ping Wheel',
    author: 'Errrrrrr',
    description: '',
    windows,
  }
}

describe('widget usage store', () => {
  test('the wanted window is used when present', () => {
    const found = statsFor(widget({ '30d': stats({ players: 42 }) }), '30d')
    expect(found?.window).toBe('30d')
    expect(found?.stats.players).toBe(42)
  })

  test('a withheld window falls back to the widest available', () => {
    // The k-anonymity floor applies inside each window, so a widget can be
    // absent from the week and present in the year. Blanking the card would
    // read as "unused" rather than "withheld here".
    const found = statsFor(
      widget({ '90d': stats(), all: stats({ players: 9 }) }),
      '7d',
    )
    expect(found?.window).toBe('all')
    expect(found?.stats.players).toBe(9)
  })

  test('the fallback does not depend on key order', () => {
    const insertedNarrowLast = widget({
      all: stats({ players: 1 }),
      '7d': stats({ players: 2 }),
    })
    expect(statsFor(insertedNarrowLast, '365d')?.window).toBe('all')
  })

  test('a widget with no windows at all has nothing to show', () => {
    expect(statsFor(widget({}), '30d')).toBeNull()
  })

  test('a partly harvested window is not presented as representative', () => {
    expect(isRepresentative(stats({ coverage: 0.04 }))).toBe(false)
    expect(isRepresentative(stats({ coverage: 1 }))).toBe(true)
  })

  test('install-and-keep is separable from install-and-forget', () => {
    expect(disabledOnly(stats({ players: 100, players_active: 80 }))).toBe(20)
    expect(disabledOnly(stats({ players: 5, players_active: 9 }))).toBe(0)
  })

  test('the window order matches what the pipeline publishes', () => {
    expect([...WINDOW_ORDER]).toEqual(['7d', '30d', '90d', '365d', 'all'])
  })
})

function published(over: Partial<Usage> = {}): Usage {
  return {
    generated_at: '2026-09-16T04:00:00+00:00',
    policy_version: 'pve_widget_harvest_v1',
    windows: ['30d'],
    widgets: [widget({ '30d': stats() })],
    ...over,
  }
}

/**
 * The store holds the document in module state, so each test imports its own
 * copy rather than inheriting the last one's.
 */
async function fresh() {
  vi.resetModules()
  return await import('./widgets')
}

describe('asking for the document', () => {
  beforeEach(() => {
    asked.mockReset()
  })

  test('one request, however many callers arrive together', async () => {
    asked.mockResolvedValue(published())
    const store = await fresh()

    await Promise.all([store.loadWidgetUsage(), store.loadWidgetUsage()])
    await store.loadWidgetUsage()

    expect(asked.mock.calls.length).toBe(1)
    expect(store.usage()?.widgets.length).toBe(1)
  })

  test('a failure is not kept, so opening the page again tries again', async () => {
    // Rust holds its own failure for as long as the service asked to be left
    // alone, so asking again costs a call into Rust and no request at all --
    // and the numbers turn up without the app being restarted.
    asked.mockResolvedValueOnce(null)
    const store = await fresh()

    await store.loadWidgetUsage()
    expect(store.usage()).toBeNull()
    expect(store.loaded()).toBe(false)

    asked.mockResolvedValueOnce(published())
    await store.loadWidgetUsage()

    expect(store.usage()?.widgets.length).toBe(1)
    expect(store.loaded()).toBe(true)
  })

  test('a document that did arrive is not asked for twice', async () => {
    asked.mockResolvedValue(published())
    const store = await fresh()

    await store.loadWidgetUsage()
    await store.loadWidgetUsage()

    expect(asked.mock.calls.length).toBe(1)
  })
})
