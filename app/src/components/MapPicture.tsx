import { Show, createEffect, createSignal } from 'solid-js'
import { mapPicture, mapThumb } from '../lib/maps'

/**
 * A map's picture in a box the stylesheet sizes, cut to that size by Rust.
 *
 * Until the cut is made, the picture as published shows in its place, scaled
 * by the webview — soft, but there — and the finished tile covers it. A map
 * the index does not know draws neither, so whatever the box paints under
 * this is the fallback.
 */
export function MapPicture(props: {
  mapName: string
  width: number
  height: number
  class?: string
  /** For a long list: asked for as the box scrolls into view. */
  lazy?: boolean
}) {
  const tile = () => mapThumb(props.mapName, props.width, props.height)
  const [broken, setBroken] = createSignal(false)

  // A box outlives the map it shows — the list's rows are virtualised, and a
  // room changes map — so a picture that failed must not condemn the next.
  createEffect(() => {
    void tile()
    setBroken(false)
  })

  const under = () => {
    const url = mapPicture(props.mapName)
    return url ? `url("${url}")` : undefined
  }

  return (
    <span
      class={props.class ? `map-pic ${props.class}` : 'map-pic'}
      style={{ 'background-image': under() }}
    >
      <Show when={broken() ? undefined : tile()}>
        {(src) => (
          <img
            src={src()}
            alt=''
            loading={props.lazy ? 'lazy' : undefined}
            decoding='async'
            onError={() => setBroken(true)}
          />
        )}
      </Show>
    </span>
  )
}
