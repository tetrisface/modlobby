import { For, Show, createMemo } from 'solid-js'
import { BoxDiff } from '../components/BoxDiff'
import { lobby } from '../store/lobby'

/** The two modoptions that decide where teams start. */
const BOX_KEYS = ['mapmetadata_startbox_override', 'mapmetadata_startboxes_set']

/**
 * Where the start boxes have been moved to this session.
 *
 * Kept per room and only in memory, which is the right lifetime: the question
 * it answers is "what changed while I was sitting here", and a room you left
 * an hour ago has no claim on your disk. The room's own option history is
 * already collected for tweaks; these are the same records, filtered.
 *
 * Silent until something moves, so a room where nobody touched the boxes shows
 * nothing at all.
 */
export function StartBoxes(props: { teams: number; mapName: string }) {
  const changes = createMemo(() =>
    (lobby.myBattle?.history ?? [])
      .filter((change) => BOX_KEYS.some((key) => change.key.endsWith(key)))
      .reverse(),
  )

  return (
    <Show when={changes().length > 0}>
      <details class='box-history'>
        <summary>
          Start boxes moved {changes().length}{' '}
          {changes().length === 1 ? 'time' : 'times'} this session
        </summary>
        <For each={changes()}>
          {(change) => (
            <BoxDiff
              title={`#${change.seq} · ${change.by ?? 'someone'}`}
              current={change.from}
              proposed={change.to}
              teams={props.teams}
              mapName={props.mapName}
            />
          )}
        </For>
      </details>
    </Show>
  )
}
