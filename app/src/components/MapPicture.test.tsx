import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { MapPicture } from './MapPicture'

// The URL is Tauri's to spell; it encodes the path whole, slash included.
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string, scheme: string) =>
    `${scheme}://${encodeURIComponent(path)}`,
}))

afterEach(cleanup)

describe('MapPicture', () => {
  test('the tile at its size, over the picture as published', () => {
    const { container } = render(() => (
      <MapPicture mapName='Comet Catcher' width={50} height={32} lazy />
    ))
    const box = container.querySelector('.map-pic') as HTMLElement
    expect(box.style.backgroundImage).toBe(
      'url("thumb://full%2FComet%20Catcher")',
    )
    const img = box.querySelector('img') as HTMLImageElement
    expect(img.getAttribute('src')).toBe('thumb://50x32%2FComet%20Catcher')
    expect(img.loading).toBe('lazy')
  })

  test('a tile that fails is dropped; the next map gets its own chance', () => {
    const [name, setName] = createSignal('Gone')
    const { container } = render(() => (
      <MapPicture mapName={name()} width={50} height={32} />
    ))
    fireEvent.error(container.querySelector('img')!)
    expect(container.querySelector('img')).toBeNull()
    setName('Here')
    expect(container.querySelector('img')?.getAttribute('src')).toBe(
      'thumb://50x32%2FHere',
    )
  })

  test('nothing for no map', () => {
    const { container } = render(() => (
      <MapPicture mapName='' width={50} height={32} class='col-thumb' />
    ))
    const box = container.querySelector('.map-pic.col-thumb') as HTMLElement
    expect(box.style.backgroundImage).toBe('')
    expect(box.querySelector('img')).toBeNull()
  })
})
