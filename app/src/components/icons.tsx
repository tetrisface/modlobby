import { For, Match, Show, Switch } from 'solid-js'
import type { BattleStatusView } from '../ipc/bindings/BattleStatusView'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { UserStatusView } from '../ipc/bindings/UserStatusView'
import { downloadFraction } from '../lib/download'

/**
 * One chevron per rank within its half; the stack grows upward. `bases` are
 * the arm-end y of each chevron, `rise` how far the apex sits above them.
 * `width` strokes the outline of ranks 1-4; `band` is the thickness of the
 * solid version for ranks 5-8: the stack pitch less a 0.9 hairline, so the
 * bands stay countable at 18px.
 */
const CHEVRONS = [
  { bases: [13], rise: 3.4, width: 1.9, band: 3.5 },
  { bases: [15, 10.6], rise: 3.4, width: 1.9, band: 3.5 },
  { bases: [16.4, 12.4, 8.4], rise: 3.2, width: 1.8, band: 3.1 },
  { bases: [17.4, 13.9, 10.4, 6.9], rise: 2.9, width: 1.6, band: 2.6 },
]

const fixed = (n: number) => n.toFixed(2)

const outline = (bases: number[], rise: number) =>
  bases.map((base) => `M5 ${base} l5 -${rise} l5 ${rise}`).join(' ')

/** Each band is the outline's centre line thickened to `band`, closed. */
const solid = (bases: number[], rise: number, band: number) =>
  bases
    .map((base) => {
      const top = base - band / 2
      const bottom = base + band / 2
      return (
        `M5 ${fixed(top)} L10 ${fixed(top - rise)} L15 ${fixed(top)} ` +
        `V${fixed(bottom)} L10 ${fixed(bottom - rise)} L5 ${fixed(bottom)} Z`
      )
    })
    .join(' ')

/** Top-right corner, clear of the lowest chevron. */
const SHIELD =
  'M15.4 2.6 L18.8 3.9 V6.6 C18.8 9.1 17.3 10.8 15.4 11.6 C13.5 10.8 12 9.1 12 6.6 V3.9 Z'

/**
 * Chobby's battle-room icon vocabulary, redrawn as vector.
 *
 * The column order and every rule below are `api_user_handler.lua`'s — status,
 * country, rank, skill, faction, name — because that grammar is what players
 * already read without looking. Only the artwork is ours.
 *
 * Mounted once near the root; every icon is a `<use>` of this sprite, except
 * the sync arrow, whose flow and fill are state and so is drawn inline
 * (`SyncIcon`).
 */
export function IconSprite() {
  return (
    <svg class='sprite' aria-hidden='true'>
      <defs>
        {/* Status. The ladder is `getUserStatusImage`'s own comment. */}
        <symbol id='st-swords' viewBox='0 0 20 20'>
          <path
            d='M4.2 16.2 L13.6 5.4 M15.8 16.2 L6.4 5.4'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-linecap='round'
          />
          <path
            d='M12.2 3.6 h3.6 v3.4 M7.8 3.6 h-3.6 v3.4'
            fill='none'
            stroke='currentColor'
            stroke-width='1.5'
            stroke-linejoin='round'
          />
        </symbol>
        <symbol id='st-unready' viewBox='0 0 20 20'>
          <path
            d='M5.4 5.4 l9.2 9.2 M14.6 5.4 l-9.2 9.2'
            stroke='currentColor'
            stroke-width='2.1'
            stroke-linecap='round'
          />
        </symbol>
        <symbol id='st-ready' viewBox='0 0 20 20'>
          <path
            d='M4.4 10.4 l3.9 3.9 l7.3 -8.2'
            fill='none'
            stroke='currentColor'
            stroke-width='2.2'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>

        {/* Rank. Eight ranks, four counts, two weights: the chevron count is
            the rank within its half, outlined for 1-4 and solid for 5-8. The
            halves are also silver and gold (see `.rank.lower`), so the weight
            does not carry the whole distinction on its own. */}
        <For each={CHEVRONS}>
          {(chevron, index) => (
            <>
              <symbol id={`chev${index() + 1}`} viewBox='0 0 20 20'>
                <path
                  d={outline(chevron.bases, chevron.rise)}
                  fill='none'
                  stroke='currentColor'
                  stroke-width={chevron.width}
                  stroke-linecap='round'
                  stroke-linejoin='round'
                />
              </symbol>
              <symbol id={`chev${index() + 1}-solid`} viewBox='0 0 20 20'>
                <path
                  d={solid(chevron.bases, chevron.rise, chevron.band)}
                  fill='currentColor'
                />
              </symbol>
            </>
          )}
        </For>
        {/* Moderator: Chobby's shield in the corner. The mask cuts a margin out
            of the chevrons behind, so the shield reads as its own mark. */}
        <symbol id='rank-shield' viewBox='0 0 20 20'>
          <path d={SHIELD} fill='currentColor' />
        </symbol>
        <mask
          id='rank-shield-cut'
          maskUnits='userSpaceOnUse'
          x='0'
          y='0'
          width='20'
          height='20'
        >
          <rect width='20' height='20' fill='white' />
          <path
            d={SHIELD}
            fill='black'
            stroke='black'
            stroke-width='2.6'
            stroke-linejoin='round'
          />
        </mask>
        <symbol id='rank-bot' viewBox='0 0 20 20'>
          <rect
            x='4.5'
            y='7'
            width='11'
            height='8.5'
            rx='2'
            fill='none'
            stroke='currentColor'
            stroke-width='1.5'
          />
          <path
            d='M10 7 V4'
            stroke='currentColor'
            stroke-width='1.5'
            stroke-linecap='round'
          />
          <circle cx='10' cy='3.2' r='1.1' fill='currentColor' />
          <circle cx='7.7' cy='11' r='1' fill='currentColor' />
          <circle cx='12.3' cy='11' r='1' fill='currentColor' />
        </symbol>

        {/* Marks after the name, Chobby's slot for them: boss and away. Flat
            strokes like everything else here, not Chobby's shaded bitmaps. */}
        <symbol id='mark-boss' viewBox='0 0 20 20'>
          <path
            d='M3.6 15.2 L2.8 6.4 L7 9.8 L10 4.4 L13 9.8 L17.2 6.4 L16.4 15.2 Z'
            fill='none'
            stroke='currentColor'
            stroke-width='1.6'
            stroke-linejoin='round'
          />
        </symbol>
        <symbol id='mark-away' viewBox='0 0 20 20'>
          <path
            d='M9.6 4.6 h6.6 l-6.6 7.2 h6.6 M3.4 11.6 h4.4 l-4.4 4.6 h4.4'
            fill='none'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>

        {/* Faction. The one coloured mark on a row, because `side` is chosen
            and known — unlike the colour the engine assigns at game start. */}
        <symbol id='side-armada' viewBox='0 0 20 20'>
          <path
            d='M10 2.6 L17.6 17.2 H2.4 Z'
            fill='none'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-linejoin='round'
          />
          <path d='M10 8.4 L13.4 14.6 H6.6 Z' fill='currentColor' />
        </symbol>
        <symbol id='side-cortex' viewBox='0 0 20 20'>
          <path
            d='M10 2.4 L17 6.3 V13.7 L10 17.6 L3 13.7 V6.3 Z'
            fill='none'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-linejoin='round'
          />
          <circle cx='10' cy='10' r='2.7' fill='currentColor' />
        </symbol>
        <symbol id='side-legion' viewBox='0 0 20 20'>
          <path
            d='M10 2.6 L17.4 10 L10 17.4 L2.6 10 Z'
            fill='none'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-linejoin='round'
          />
          <path d='M10 6.6 L13.4 10 L10 13.4 L6.6 10 Z' fill='currentColor' />
        </symbol>
        {/* Actions on a tweak slot or a preset row: copy, edit, delete. */}
        <symbol id='act-copy' viewBox='0 0 20 20'>
          <rect
            x='7'
            y='7'
            width='9.5'
            height='9.5'
            rx='1.5'
            fill='none'
            stroke='currentColor'
            stroke-width='1.6'
          />
          <path
            d='M13 6.2 V5 a1.5 1.5 0 0 0 -1.5 -1.5 H5 A1.5 1.5 0 0 0 3.5 5 v6.5 A1.5 1.5 0 0 0 5 13 h1.2'
            fill='none'
            stroke='currentColor'
            stroke-width='1.6'
            stroke-linecap='round'
          />
        </symbol>
        <symbol id='act-pen' viewBox='0 0 20 20'>
          <path
            d='M4 16 l0.9 -3.6 L13.4 3.9 a1.6 1.6 0 0 1 2.3 0 l0.4 0.4 a1.6 1.6 0 0 1 0 2.3 L7.6 15.1 Z'
            fill='none'
            stroke='currentColor'
            stroke-width='1.6'
            stroke-linejoin='round'
          />
          <path
            d='M11.8 5.5 l2.7 2.7'
            stroke='currentColor'
            stroke-width='1.6'
            stroke-linecap='round'
          />
        </symbol>
        <symbol id='act-trash' viewBox='0 0 20 20'>
          <path
            d='M4 6 h12 M8 6 V4.2 h4 V6 M5.5 6 l0.8 10 h7.4 l0.8 -10 M8.4 9 v4.6 M11.6 9 v4.6'
            fill='none'
            stroke='currentColor'
            stroke-width='1.6'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>
        {/* Two arrows chasing each other round a circle: try the connection again. */}
        <symbol id='act-reconnect' viewBox='0 0 20 20'>
          <path
            d='M17 4.9 v3.8 h-3.8 M3 15.1 v-3.8 h3.8 M4.6 8.1 a5.7 5.7 0 0 1 9.45 -2.14 L17 8.7 M3 11.3 l2.95 2.77 A5.7 5.7 0 0 0 15.4 11.9'
            fill='none'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>
        <symbol id='side-random' viewBox='0 0 20 20'>
          <path
            d='M10 2.6 L17.4 10 L10 17.4 L2.6 10 Z'
            fill='none'
            stroke='currentColor'
            stroke-width='1.7'
            stroke-dasharray='2.6 2.2'
            stroke-linejoin='round'
          />
        </symbol>
      </defs>
    </svg>
  )
}

/** A symbol inside a control that already says what it does; no title of its own. */
export function Glyph(props: { id: string }) {
  return (
    <svg class='icon act' aria-hidden='true'>
      <use href={`#${props.id}`} />
    </svg>
  )
}

function Icon(props: { id: string; class: string; label: string }) {
  return (
    <svg class={`icon ${props.class}`} role='img'>
      <title>{props.label}</title>
      <use href={`#${props.id}`} />
    </svg>
  )
}

/**
 * The download arrow: a shaft whose dashes run downward (CSS), over a glass
 * that fills from the bottom as far as `fraction` says. Same artwork the
 * sprite's status icons use, drawn inline because a `<use>` instance cannot
 * be reached by document selectors, and the fill is per-instance state.
 *
 * `running` colours it as work in progress; `fraction` null draws no glass,
 * for an arrow whose progress is not ours to know.
 */
export function SyncIcon(props: {
  fraction: number | null
  running: boolean
  label: string
}) {
  const height = () => 20 * (props.fraction ?? 0)
  return (
    <svg
      class='icon status sync'
      classList={{ running: props.running }}
      viewBox='0 0 20 20'
      role='img'
    >
      <title>{props.label}</title>
      <Show when={props.fraction !== null}>
        <rect
          class='glass'
          x='0'
          y={20 - height()}
          width='20'
          height={height()}
          rx='2'
        />
      </Show>
      <g
        fill='none'
        stroke='currentColor'
        stroke-width='1.8'
        stroke-linecap='round'
        stroke-linejoin='round'
      >
        <path class='shaft' d='M10 3.4 V11.6' />
        <path d='M6.3 8.4 L10 12.1 L13.7 8.4' />
        <path d='M4.4 15.4 h11.2' />
      </g>
    </svg>
  )
}

/**
 * Players only — Chobby hides it outright for spectators, and so do we.
 * In game beats unsynced beats not-ready, in that order.
 *
 * `download` is our own pr-downloader run, passed for our row alone: it is
 * what lets the arrow fill and say a percentage. Other players' arrows only
 * say that they are unsynced, which is all the protocol tells us.
 */
export function StatusIcon(props: {
  status: UserStatusView
  battle: BattleStatusView
  download?: DownloadStatus
}) {
  const fraction = () =>
    props.download ? downloadFraction(props.download) : null

  const syncLabel = () => {
    const at = fraction()
    return at === null
      ? 'Downloading content'
      : `Downloading ${Math.round(at * 100)}%`
  }

  return (
    <Switch>
      <Match when={props.status.inGame}>
        <Icon id='st-swords' class='status ingame' label='In game' />
      </Match>
      <Match when={props.battle.sync === 'unsynced'}>
        <SyncIcon
          fraction={fraction()}
          running={props.download?.state === 'running'}
          label={syncLabel()}
        />
      </Match>
      <Match when={props.battle.ready}>
        <Icon id='st-ready' class='status ready' label='Ready' />
      </Match>
      <Match when={true}>
        <Icon id='st-unready' class='status unready' label='Not ready' />
      </Match>
    </Switch>
  )
}

/**
 * What Chobby stacks after the name (`GetUserStatusImages`): the room's boss
 * and anyone marked away. Both may show at once; neither is a column.
 */
export function Marks(props: { status: UserStatusView; boss: boolean }) {
  return (
    <span class='marks'>
      <Show when={props.boss}>
        <Icon id='mark-boss' class='mark boss' label='Boss' />
      </Show>
      <Show when={props.status.away}>
        <Icon id='mark-away' class='mark away' label='Away' />
      </Show>
    </span>
  )
}

/** `rank` is client-status bits 2-4 (0-7); Chobby numbers the same thing 1-8. */
export function RankIcon(props: { status: UserStatusView }) {
  const level = () => props.status.rank + 1
  const upper = () => level() > 4
  const chevrons = () => (upper() ? level() - 4 : level())
  const label = () =>
    `Rank ${level()}${props.status.moderator ? ' · moderator' : ''}`

  return (
    <Show
      when={!props.status.bot}
      fallback={<Icon id='rank-bot' class='rank bot' label='Autohost' />}
    >
      <svg
        class='icon rank'
        classList={{ lower: !upper() }}
        viewBox='0 0 20 20'
        role='img'
      >
        <title>{label()}</title>
        <use
          href={`#chev${chevrons()}${upper() ? '-solid' : ''}`}
          mask={props.status.moderator ? 'url(#rank-shield-cut)' : undefined}
        />
        <Show when={props.status.moderator}>
          <use href='#rank-shield' class='shield' />
        </Show>
      </svg>
    </Show>
  )
}

const SIDES = [
  { id: 'side-armada', class: 'armada', label: 'Armada' },
  { id: 'side-cortex', class: 'cortex', label: 'Cortex' },
  { id: 'side-random', class: 'random', label: 'Random faction' },
  { id: 'side-legion', class: 'legion', label: 'Legion' },
]

/** Battle-status bits 24-27. Hidden for spectators, as in Chobby. */
export function SideIcon(props: { side: number }) {
  const side = () => SIDES[props.side]

  return (
    <Show when={side()}>
      {(s) => (
        <Icon id={s().id} class={`side ${s().class}`} label={s().label} />
      )}
    </Show>
  )
}

/**
 * `ADDUSER`'s country field. Drawn from `flag-icons`, never emoji: Windows
 * ships no regional-indicator glyphs, so a flag emoji renders as two boxed
 * letters on the platform this app targets. teiserver sends `??` when unset.
 */
export function Flag(props: { country: string }) {
  const code = () => props.country.trim().toLowerCase()
  const known = () => /^[a-z]{2}$/.test(code())

  return (
    <Show
      when={known()}
      fallback={<span class='flag unknown' title='Country unknown' />}
    >
      <span
        class={`flag fi fi-${code()}`}
        title={props.country.toUpperCase()}
        role='img'
      />
    </Show>
  )
}
