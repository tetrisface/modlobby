import { Show, createEffect, createResource, createSignal } from 'solid-js'
import { api } from '../ipc/client'
import { lobby } from '../store/lobby'

/**
 * What BAR's PvE Stats service says this room scores.
 *
 * The same numbers the in-game widget shows, asked for while there is still
 * time to change the answer — which is when "this is going to be a massacre"
 * is worth knowing.
 *
 * Nothing is asked for a room that is not PvE, and nothing about a person is
 * ever sent: the service's own note is that the estimate represents a generic
 * team and does not use the identities or ratings of whoever is present.
 */
export function PveScore() {
  /** Re-asked when the room's settings change, not on a timer. */
  const [key, setKey] = createSignal(0)
  createEffect(() => {
    // Reading these is what subscribes us to them.
    void lobby.myBattle?.scriptTags
    void lobby.myBattle?.id
    setKey((held) => held + 1)
  })

  const [score] = createResource(key, () => api.pveScore().catch(() => null))

  const percent = (value: number | null) =>
    value === null ? '—' : `${Math.round(value * 100)}%`

  return (
    <Show when={score()}>
      {(held) => (
        <div class='pve-score'>
          <span class='filter-label'>PvE</span>

          <Show
            when={held().challenge !== null}
            fallback={
              <span
                class='muted'
                title='This setup has not been placed among played games yet.'
              >
                unplaced
              </span>
            }
          >
            <span
              class='pve-figure'
              title='Absolute difficulty on a 0-34 scale. 17 is an estimated even game for a representative human team; higher is harder.'
            >
              Challenge <b>{held().challenge?.toFixed(1)}</b>
            </span>
          </Show>

          <Show when={held().winChance !== null}>
            <span
              class='pve-figure'
              title='Estimated chance a representative current BAR human team wins this map and setup. The people in this room are not part of that estimate.'
            >
              Win <b>{percent(held().winChance)}</b>
            </span>
          </Show>

          <Show when={held().percentile !== null}>
            <span
              class='muted'
              title='Where this setup sits among eligible played games for this opponent.'
            >
              harder than {Math.round(held().percentile ?? 0)}%
            </span>
          </Show>

          <Show when={held().bestEffort}>
            <span
              class='chip warn'
              title='This room uses settings the service has not catalogued, so these are best-effort estimates rather than an exact match.'
            >
              best effort
            </span>
          </Show>
        </div>
      )}
    </Show>
  )
}
