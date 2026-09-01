import { describe, expect, it } from 'vitest'
import { sized } from './maps'

// The shape the map index publishes: an imagor transform with the size in the
// path and the filters after it.
const PUBLISHED =
  'https://api.bar-rts.com/i/unsafe/fit-in/1024x1024/filters:quality(75):format(webp)/maps/some_map.jpg'

describe('asking for a picture at the size it will be drawn', () => {
  it('puts the wanted size into the transform', () => {
    expect(sized(PUBLISHED, 52)).toContain('/fit-in/52x52/')
    expect(sized(PUBLISHED, 52)).not.toContain('1024x1024')
  })

  it('raises the quality, since the image is now small', () => {
    expect(sized(PUBLISHED, 52)).toContain('quality(90)')
    expect(sized(PUBLISHED, 52)).not.toContain('quality(75)')
  })

  it('leaves the rest of the URL alone', () => {
    expect(sized(PUBLISHED, 256)).toBe(
      'https://api.bar-rts.com/i/unsafe/fit-in/256x256/filters:quality(90):format(webp)/maps/some_map.jpg',
    )
  })

  it('can be applied to its own output', () => {
    expect(sized(sized(PUBLISHED, 52), 384)).toBe(sized(PUBLISHED, 384))
  })

  it('returns a URL of another shape untouched', () => {
    const plain = 'https://example.invalid/maps/some_map.png'
    expect(sized(plain, 52)).toBe(plain)
  })
})
