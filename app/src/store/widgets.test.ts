import { describe, expect, test } from 'vitest'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import {
  WINDOW_ORDER,
  disabledOnly,
  isRepresentative,
  statsFor,
} from './widgets'

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
