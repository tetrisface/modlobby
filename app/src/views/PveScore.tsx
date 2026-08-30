import {
  Show,
  createEffect,
  createResource,
  createSignal,
  onCleanup,
} from 'solid-js'
import { Thinking } from '../components/Thinking'
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
/** How long the room has to stop changing before it is worth asking again. */
const QUIET_FOR = 3000

export function PveScore() {
  /**
   * Re-asked when the room settles, not on every change.
   *
   * A host applying a preset changes a hundred settings in a couple of
   * minutes, and each one arrives as its own script tag. Asking per change
   * would put a hundred requests on somebody else's service to answer a
   * question whose answer only matters once the changes stop.
   */
  const [settled, setSettled] = createSignal(0)
  let pending: ReturnType<typeof setTimeout> | undefined

  createEffect(() => {
    // Reading these is what subscribes us to them.
    void lobby.myBattle?.scriptTags
    void lobby.myBattle?.id
    clearTimeout(pending)
    pending = setTimeout(() => setSettled((held) => held + 1), QUIET_FOR)
  })
  onCleanup(() => clearTimeout(pending))

  /**
   * Whether this room has a PvE opponent in it at all.
   *
   * Decided here as well as in Rust, which stays authoritative for what is
   * sent. This one only decides whether to ask — without it a room with no AI
   * in it flashes "PvE" and a row of dots before the answer comes back null,
   * and every settings change in every ordinary room costs a round trip to
   * find out nothing.
   */
  const isPve = () => {
    const id = lobby.myBattle?.id
    const room = id === undefined ? undefined : lobby.battles[id]
    const names = (room?.bots ?? [])
      .map((bot) => bot.ai.toLowerCase())
      .join(' ')
    const raptors = names.includes('raptor')
    const scavengers = names.includes('scavenger')
    // Both at once is not a setup the model knows, so it is not asked about.
    if (raptors && scavengers) return false
    return raptors || scavengers || names.includes('barb')
  }

  const [score] = createResource(
    () => (isPve() ? settled() : undefined),
    () => api.pveScore().catch(() => null),
  )

  const percent = (value: number | null) =>
    value === null ? '—' : `${Math.round(value * 100)}%`

  return (
    <Show when={score.loading || score()}>
      <div class='pve-score'>
        <span class='filter-label'>PvE</span>

        {/* Asking takes a moment and the answer is a bare number, so without
            this it appears from nowhere and reads as a glitch. */}
        <Show when={score.loading}>
          <Thinking title='asking how hard this setup is' />
        </Show>

        <Show when={score()}>
          {(held) => (
            <>
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
            </>
          )}
        </Show>
      </div>
    </Show>
  )
}
