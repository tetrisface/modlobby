import { describe, expect, test } from 'vitest'
import { downloadFraction } from './download'

describe('downloadFraction', () => {
  test('is null when nothing is running', () => {
    expect(downloadFraction({ state: 'idle' })).toBeNull()
    expect(
      downloadFraction({ state: 'failed', what: 'map', reason: 'x' }),
    ).toBeNull()
    expect(downloadFraction({ state: 'done', what: 'map' })).toBeNull()
  })

  test('is null until pr-downloader states a size', () => {
    expect(
      downloadFraction({ state: 'running', what: 'map', current: 0, total: 0 }),
    ).toBeNull()
  })

  test('is the fraction of a sized download, capped at one', () => {
    expect(
      downloadFraction({
        state: 'running',
        what: 'map',
        current: 4,
        total: 10,
      }),
    ).toBe(0.4)
    expect(
      downloadFraction({
        state: 'running',
        what: 'map',
        current: 12,
        total: 10,
      }),
    ).toBe(1)
  })
})
