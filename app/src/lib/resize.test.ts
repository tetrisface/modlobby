import { describe, expect, test } from 'vitest'
import {
  clamp,
  dragHeight,
  dragWidth,
  readFlag,
  readWidth,
  splitBounds,
  writeFlag,
  writeWidth,
  type WidthStore,
} from './resize'

const bounds = { min: 420, max: 900 }

function memory(initial: Record<string, string> = {}): WidthStore & {
  data: Record<string, string>
} {
  const data = { ...initial }
  return {
    data,
    getItem: (key) => data[key] ?? null,
    setItem: (key, value) => {
      data[key] = value
    },
  }
}

describe('clamp', () => {
  test('keeps a width inside its bounds', () => {
    expect(clamp(600, bounds)).toBe(600)
    expect(clamp(100, bounds)).toBe(420)
    expect(clamp(5000, bounds)).toBe(900)
  })
})

describe('dragWidth', () => {
  test('moving the pointer left widens a pane whose grip is on its left edge', () => {
    expect(dragWidth(556, 800, 700, bounds)).toBe(656)
    expect(dragWidth(556, 800, 900, bounds)).toBe(456)
  })

  test('never leaves the bounds however far the pointer goes', () => {
    expect(dragWidth(556, 800, 0, bounds)).toBe(900)
    expect(dragWidth(556, 800, 3000, bounds)).toBe(420)
  })

  test('rounds to whole pixels', () => {
    expect(dragWidth(556, 800, 799.6, bounds)).toBe(556)
  })
})

describe('readWidth', () => {
  test('returns the saved number', () => {
    expect(readWidth(memory({ w: '640' }), 'w')).toBe(640)
  })

  test('is null when nothing is saved, or when what is saved is not a width', () => {
    expect(readWidth(memory(), 'w')).toBeNull()
    expect(readWidth(memory({ w: 'wide' }), 'w')).toBeNull()
    expect(readWidth(memory({ w: '-3' }), 'w')).toBeNull()
    expect(readWidth(memory({ w: 'Infinity' }), 'w')).toBeNull()
  })

  test('is null without storage, or with storage that throws', () => {
    expect(readWidth(null, 'w')).toBeNull()
    const broken: WidthStore = {
      getItem: () => {
        throw new Error('denied')
      },
      setItem: () => {},
    }
    expect(readWidth(broken, 'w')).toBeNull()
  })
})

describe('writeWidth', () => {
  test('saves whole pixels as text', () => {
    const store = memory()
    writeWidth(store, 'w', 640.4)
    expect(store.data.w).toBe('640')
  })

  test('survives missing or refusing storage', () => {
    expect(() => writeWidth(null, 'w', 1)).not.toThrow()
    const broken: WidthStore = {
      getItem: () => null,
      setItem: () => {
        throw new Error('quota')
      },
    }
    expect(() => writeWidth(broken, 'w', 1)).not.toThrow()
  })
})

describe('dragHeight', () => {
  test('moving the pointer down makes a pane whose grip is on its bottom edge taller', () => {
    expect(dragHeight(400, 500, 560, { min: 64, max: 700 })).toBe(460)
    expect(dragHeight(400, 500, 440, { min: 64, max: 700 })).toBe(340)
  })

  test('never leaves the bounds however far the pointer goes', () => {
    expect(dragHeight(400, 500, 5000, { min: 64, max: 700 })).toBe(700)
    expect(dragHeight(400, 500, -5000, { min: 64, max: 700 })).toBe(64)
  })
})

describe('splitBounds', () => {
  test('leaves the neighbour its share', () => {
    expect(splitBounds(1000, 176, 64)).toEqual({ min: 64, max: 824 })
  })

  test('a pane too small for the share still has its minimum', () => {
    expect(splitBounds(100, 176, 64)).toEqual({ min: 64, max: 64 })
  })
})

describe('flags', () => {
  test('round-trip, and read as off when unset or unreadable', () => {
    const store = memory()
    expect(readFlag(store, 'k')).toBe(false)
    writeFlag(store, 'k', true)
    expect(readFlag(store, 'k')).toBe(true)
    writeFlag(store, 'k', false)
    expect(readFlag(store, 'k')).toBe(false)
    expect(readFlag(null, 'k')).toBe(false)
  })
})
