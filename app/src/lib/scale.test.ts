import { describe, expect, test } from 'vitest'
import { bounds, bucket, clamp, derived, scaleFor, step } from './scale'

describe('which screen this is', () => {
  test('two displays of different sizes are different buckets', () => {
    expect(bucket(1920, 1080)).not.toBe(bucket(2560, 1440))
  })

  test('a display that moves by a taskbar is still the same display', () => {
    expect(bucket(1920, 1080)).toBe(bucket(1920, 1040))
  })
})

describe('the range worth offering', () => {
  test('never below full size, however small the screen', () => {
    expect(bounds(800, 480).max).toBe(100)
  })

  test('a small laptop still has room to grow', () => {
    expect(bounds(1280, 720).max).toBe(133)
  })

  test("Chobby's ceiling: 200% at 1080p, 400% at 4k", () => {
    expect(bounds(1920, 1080).max).toBe(200)
    expect(bounds(3840, 2160).max).toBe(400)
  })

  test('the floor keeps body text readable', () => {
    expect(bounds(1920, 1080).min).toBe(50)
  })
})

describe('a screen nobody has chosen a size for', () => {
  test('1080p is drawn as it is', () => {
    expect(derived(1920, 1080)).toBe(100)
  })

  test('a 2.5k screen is drawn larger without anyone asking', () => {
    expect(derived(2560, 1440)).toBe(110)
  })

  test('4k larger still', () => {
    expect(derived(3840, 2160)).toBe(125)
  })
})

describe('the size to draw at', () => {
  test('what was chosen for this screen wins over what suits it', () => {
    const saved = { [bucket(2560, 1440)]: 150 }
    expect(scaleFor(saved, 2560, 1440)).toBe(150)
  })

  test('a choice made on another screen does not follow you to this one', () => {
    const saved = { [bucket(3840, 2160)]: 200 }
    expect(scaleFor(saved, 1920, 1080)).toBe(derived(1920, 1080))
  })

  test('a hand-edited file asking for more than the screen allows is held back', () => {
    const saved = { [bucket(1920, 1080)]: 900 }
    expect(scaleFor(saved, 1920, 1080)).toBe(200)
  })

  test('and one asking for nothing legible is held up', () => {
    const saved = { [bucket(1920, 1080)]: 1 }
    expect(scaleFor(saved, 1920, 1080)).toBe(50)
  })
})

describe('one notch', () => {
  test('moves by ten either way', () => {
    expect(step(100, 1)).toBe(110)
    expect(step(100, -1)).toBe(90)
  })

  test('lands on a round number from wherever it started', () => {
    expect(step(113, 1)).toBe(120)
    expect(step(113, -1)).toBe(100)
  })

  test('clamping is the caller-s job, so a notch past the end still moves', () => {
    expect(clamp(step(200, 1), 1920, 1080)).toBe(200)
  })
})
