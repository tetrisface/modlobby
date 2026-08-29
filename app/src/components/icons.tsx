import { Show } from 'solid-js'
import type { BattleStatusView } from '../ipc/bindings/BattleStatusView'
import type { UserStatusView } from '../ipc/bindings/UserStatusView'

/**
 * Chobby's battle-room icon vocabulary, redrawn as vector.
 *
 * The column order and every rule below are `api_user_handler.lua`'s — status,
 * country, rank, skill, faction, name — because that grammar is what players
 * already read without looking. Only the artwork is ours.
 *
 * Mounted once near the root; every icon is a `<use>` of this sprite.
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
        <symbol id='st-sync' viewBox='0 0 20 20'>
          <path
            d='M10 3.4 V11.6 M6.3 8.4 L10 12.1 L13.7 8.4 M4.4 15.4 h11.2'
            fill='none'
            stroke='currentColor'
            stroke-width='1.8'
            stroke-linecap='round'
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

        {/* Rank. Eight ranks, four shapes: the chevron count is the rank
            within its metal, the metal (steel 1-4 / gold 5-8) is the half. */}
        <symbol id='chev1' viewBox='0 0 20 20'>
          <path
            d='M5 13 l5 -3.4 l5 3.4'
            fill='none'
            stroke='currentColor'
            stroke-width='1.9'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>
        <symbol id='chev2' viewBox='0 0 20 20'>
          <path
            d='M5 15 l5 -3.4 l5 3.4 M5 10.6 l5 -3.4 l5 3.4'
            fill='none'
            stroke='currentColor'
            stroke-width='1.9'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>
        <symbol id='chev3' viewBox='0 0 20 20'>
          <path
            d='M5 16.4 l5 -3.2 l5 3.2 M5 12.4 l5 -3.2 l5 3.2 M5 8.4 l5 -3.2 l5 3.2'
            fill='none'
            stroke='currentColor'
            stroke-width='1.8'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>
        <symbol id='chev4' viewBox='0 0 20 20'>
          <path
            d='M5 17.4 l5 -2.9 l5 2.9 M5 13.9 l5 -2.9 l5 2.9 M5 10.4 l5 -2.9 l5 2.9 M5 6.9 l5 -2.9 l5 2.9'
            fill='none'
            stroke='currentColor'
            stroke-width='1.6'
            stroke-linecap='round'
            stroke-linejoin='round'
          />
        </symbol>
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

function Icon(props: { id: string; class: string; label: string }) {
  return (
    <svg class={`icon ${props.class}`} role='img'>
      <title>{props.label}</title>
      <use href={`#${props.id}`} />
    </svg>
  )
}

/**
 * Players only — Chobby hides it outright for spectators, and so do we.
 * In game beats unsynced beats not-ready, in that order.
 */
export function StatusIcon(props: {
  status: UserStatusView
  battle: BattleStatusView
}) {
  const shown = () => {
    if (props.status.inGame)
      return { id: 'st-swords', class: 'ingame', label: 'In game' }
    if (props.battle.sync === 'unsynced')
      return { id: 'st-sync', class: 'sync', label: 'Downloading content' }
    if (props.battle.ready)
      return { id: 'st-ready', class: 'ready', label: 'Ready' }
    return { id: 'st-unready', class: 'unready', label: 'Not ready' }
  }

  return (
    <Icon
      id={shown().id}
      class={`status ${shown().class}`}
      label={shown().label}
    />
  )
}

/** `rank` is client-status bits 2-4 (0-7); Chobby numbers the same thing 1-8. */
export function RankIcon(props: { status: UserStatusView }) {
  const level = () => props.status.rank + 1
  const gold = () => level() > 4
  const chevrons = () => (gold() ? level() - 4 : level())

  return (
    <Show
      when={!props.status.bot}
      fallback={<Icon id='rank-bot' class='rank bot' label='Autohost' />}
    >
      <Icon
        id={`chev${chevrons()}`}
        class={`rank ${gold() ? 'gold' : ''} ${props.status.moderator ? 'mod' : ''}`}
        label={`Rank ${level()}${props.status.moderator ? ' · moderator' : ''}`}
      />
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
