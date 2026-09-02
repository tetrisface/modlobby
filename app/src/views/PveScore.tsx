import {
  Show,
  createEffect,
  createMemo,
  createSignal,
  on,
  onCleanup,
} from 'solid-js'
import { Thinking } from '../components/Thinking'
import type { Score } from '../ipc/bindings/Score'
import { api, describeError } from '../ipc/client'
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
export const QUIET_FOR = 3000

/**
 * Everything the answer depends on, as one string.
 *
 * Rust builds the request from the room; this only has to notice when any of
 * that changes. Reading each value here is what subscribes to it — the store
 * mutates the settings object in place, so watching the object itself would
 * never fire again after the first look.
 */
function fingerprint(): string | undefined {
  const my = lobby.myBattle
  const room = my ? lobby.battles[my.id] : undefined
  if (!my || !room) return undefined
  const settings = Object.entries(my.scriptTags)
    .filter(([key]) => key.startsWith('game/modoptions/'))
    .map(([key, value]) => `${key}=${value}`)
    .sort()
  const bots = room.bots.map((bot) => `${bot.ai}@${bot.status.handicap}`)
  const seats = room.members.map((name) => {
    const status = lobby.users[name]?.battleStatus
    return status?.player ? String(status.handicap) : ''
  })
  return [
    my.id,
    room.mapName,
    room.playerCount,
    ...bots,
    ...seats,
    ...settings,
  ].join('\n')
}

/**
 * Whether this room has a PvE opponent in it at all.
 *
 * Decided here as well as in Rust, which stays authoritative for what is
 * sent. This one only decides whether to ask — without it a room with no AI
 * in it flashes "PvE" and a row of dots before the answer comes back null,
 * and every settings change in every ordinary room costs a round trip to
 * find out nothing.
 */
function isPve(): boolean {
  const id = lobby.myBattle?.id
  const room = id === undefined ? undefined : lobby.battles[id]
  const names = (room?.bots ?? []).map((bot) => bot.ai.toLowerCase()).join(' ')
  const raptors = names.includes('raptor')
  const scavengers = names.includes('scavenger')
  // Both at once is not a setup the model knows, so it is not asked about.
  if (raptors && scavengers) return false
  return raptors || scavengers || names.includes('barb')
}

export function PveScore() {
  /** `undefined` before any answer; `null` when Rust declined to ask. */
  const [score, setScore] = createSignal<Score | null | undefined>(undefined)
  const [asking, setAsking] = createSignal(false)
  const [failure, setFailure] = createSignal<string | null>(null)

  /**
   * One question at a time.
   *
   * The service is a Lambda that takes some twenty seconds to wake up, and a
   * Lambda answers concurrent requests with concurrent cold starts. Asking
   * again while the first ask is still out does not get an answer sooner; it
   * gets two cold starts. So a change during an ask is remembered and asked
   * about once the answer is in — by which time the service is warm and the
   * follow-up takes half a second. Rust reads the room afresh for each ask,
   * so one follow-up covers any number of changes.
   */
  let inFlight = false
  let again = false
  let alive = true

  async function ask() {
    if (inFlight) {
      again = true
      return
    }
    inFlight = true
    setAsking(true)
    do {
      again = false
      try {
        const answer = await api.pveScore()
        if (!alive) return
        setScore(answer)
        setFailure(null)
      } catch (err) {
        if (!alive) return
        setFailure(describeError(err))
        console.warn('pve stats:', err)
      }
    } while (again)
    inFlight = false
    setAsking(false)
  }

  /**
   * Asked as soon as a room is in view, then again whenever it settles.
   *
   * A host applying a preset changes a hundred settings in a couple of
   * minutes, and each one arrives as its own script tag. Asking per change
   * would put a hundred requests on somebody else's service to answer a
   * question whose answer only matters once the changes stop.
   */
  let pending: ReturnType<typeof setTimeout> | undefined
  // A memo, so a change that leaves the string as it was — someone toggling
  // ready, say — does not count as the room changing.
  const room = createMemo(fingerprint)
  createEffect(
    on(room, (now, before) => {
      clearTimeout(pending)
      if (now === undefined || !isPve()) return
      const sameRoom = before?.split('\n')[0] === now.split('\n')[0]
      if (sameRoom) {
        pending = setTimeout(() => void ask(), QUIET_FOR)
        return
      }
      // A new room: whatever the last one scored is not this one's.
      setScore(undefined)
      setFailure(null)
      void ask()
    }),
  )
  onCleanup(() => {
    alive = false
    clearTimeout(pending)
  })

  const percent = (value: number | null) =>
    value === null ? '—' : `${Math.round(value * 100)}%`

  /**
   * A number's place in the row, kept while the number is being asked for.
   *
   * The dots sit where the figure will land, so the row neither disappears
   * nor jumps when an answer arrives; a figure appearing from nowhere reads
   * as a glitch.
   */
  const Slot = (props: { value: string }) => (
    <b>
      <Show
        when={!asking()}
        fallback={<Thinking title='asking how hard this setup is' />}
      >
        {props.value}
      </Show>
    </b>
  )

  return (
    <Show when={isPve() && score() !== null}>
      <div class='pve-score'>
        <span class='filter-label'>PvE</span>

        <Show
          when={!failure()}
          fallback={
            <span class='muted' title={failure() ?? ''}>
              unavailable
            </span>
          }
        >
          <span
            class='pve-figure'
            title='Absolute difficulty on a 0-34 scale. 17 is an estimated even game for a representative human team; higher is harder.'
          >
            Challenge{' '}
            <Show
              when={asking() || score()?.challenge != null}
              fallback={
                <span
                  class='muted'
                  title='This setup has not been placed among played games yet.'
                >
                  unplaced
                </span>
              }
            >
              <Slot value={(score()?.challenge?.toFixed(1) ?? '—') + '/34'} />
            </Show>
          </span>

          <span
            class='pve-figure'
            title='Estimated chance a representative current BAR human team wins this map and setup. The people in this room are not part of that estimate.'
          >
            Win <Slot value={percent(score()?.winChance ?? null)} />
          </span>

          <span
            class='pve-figure muted'
            title='Where this setup sits among eligible played games for this opponent.'
          >
            harder than{' '}
            <Slot
              value={
                score()?.percentile == null
                  ? '—'
                  : `${Math.round(score()?.percentile ?? 0)}%`
              }
            />
          </span>

          <Show when={!asking() && score()?.bestEffort}>
            <span
              class='chip warn'
              title='This room uses settings the service has not catalogued, so these are best-effort estimates rather than an exact match.'
            >
              best effort
            </span>
          </Show>
        </Show>
      </div>
    </Show>
  )
}
