import { For, Show, createResource } from 'solid-js'
import { api } from '../ipc/client'
import { centre, outline } from '../lib/boxes'
import { mapImage } from '../lib/maps'

function Panel(props: {
  label: string
  blob: string
  teams: number
  mapName: string
  tone: 'before' | 'after'
}) {
  const [image] = createResource(() => props.mapName, mapImage)
  const [polys] = createResource(
    () => [props.blob, props.teams] as const,
    ([blob, teams]) => api.decodeBoxes(blob, teams).catch(() => null),
  )

  return (
    <figure class='box-panel' classList={{ [props.tone]: true }}>
      <figcaption>{props.label}</figcaption>
      <div class='box-map'>
        <Show when={image()}>{(url) => <img src={url()} alt='' />}</Show>
        <svg viewBox='0 0 200 200' role='img'>
          <title>{props.label}</title>
          <For each={polys() ?? []}>
            {(poly, index) => (
              <g>
                <path d={outline(poly)} />
                <text x={centre(poly).x} y={centre(poly).y + 7}>
                  {index() + 1}
                </text>
              </g>
            )}
          </For>
        </svg>
      </div>
      {/* An unreadable blob and an empty one mean different things, and a vote
          is exactly when the difference matters. */}
      <span class='muted'>
        {polys.loading
          ? '…'
          : polys() === null
            ? props.blob === '' || props.blob === '0'
              ? 'none'
              : 'unreadable'
            : `${polys()?.length} boxes`}
      </span>
    </figure>
  )
}

/**
 * What a start-box change would do, drawn rather than described.
 *
 * A startbox modoption is base64url(zlib(json)) — a wall of characters that
 * tells nobody anything. Two minimaps side by side answer the only question
 * anyone actually has about a `!bSet mapmetadata_startbox_override` vote,
 * which is where the teams would end up.
 */
export function BoxDiff(props: {
  current: string
  proposed: string
  teams: number
  mapName: string
  title?: string
}) {
  return (
    <section class='box-diff'>
      <Show when={props.title}>
        <h2>{props.title}</h2>
      </Show>
      <div class='box-pair'>
        <Panel
          label='Now'
          tone='before'
          blob={props.current}
          teams={props.teams}
          mapName={props.mapName}
        />
        <span class='box-arrow' aria-hidden='true'>
          →
        </span>
        <Panel
          label='Proposed'
          tone='after'
          blob={props.proposed}
          teams={props.teams}
          mapName={props.mapName}
        />
      </div>
    </section>
  )
}
