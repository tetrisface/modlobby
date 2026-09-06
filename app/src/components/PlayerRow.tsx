import { Show } from 'solid-js'
import type { BotView } from '../ipc/bindings/BotView'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { UserView } from '../ipc/bindings/UserView'
import { type Skill, skillText, skillTier, skillTitle } from '../lib/skill'
import { Flag, Glyph, Marks, RankIcon, SideIcon, StatusIcon } from './icons'
import { showBotMenu, showPlayerMenu } from './PlayerMenu'

/**
 * One player, in Chobby's column order: status, country, rank, skill, faction,
 * name. Nothing here is coloured by team — the engine assigns player colours at
 * game start, so a pre-game team hue would be a guess. The faction is the one
 * coloured mark, because that one is chosen and known.
 *
 * `download` is our own run, for our own row: the one arrow that can fill.
 */
export function PlayerRow(props: {
  user: UserView
  skill: Skill | null
  me: boolean
  friend?: boolean
  boss?: boolean
  download?: DownloadStatus
}) {
  return (
    <Show when={props.user.battleStatus}>
      {(battle) => (
        <div class='player'>
          <StatusIcon
            status={props.user.status}
            battle={battle()}
            download={props.download}
          />
          <Flag country={props.user.country} />
          <RankIcon status={props.user.status} />
          <SkillCell skill={props.skill} />
          <SideIcon side={battle().side} />
          <span
            class='pname'
            classList={{ me: props.me, friend: props.friend }}
            onClick={(event) => showPlayerMenu(props.user.name, event)}
            onContextMenu={(event) => showPlayerMenu(props.user.name, event)}
          >
            {props.user.name}
          </span>
          <Marks status={props.user.status} boss={props.boss ?? false} />
        </div>
      )}
    </Show>
  )
}

/**
 * An AI seat. It holds a team but has no lobby account behind it.
 *
 * `onRemove` comes for our own AIs only, and puts removing where a player's
 * actions are — behind the name, on either click — and behind a bin that shows
 * on hover. Another player's AI gets neither, since the server would refuse.
 */
export function BotRow(props: {
  bot: BotView
  onRemove?: () => Promise<void>
  /** Offered where the room can tell an AI anything about itself. */
  onOptions?: () => void
}) {
  const menu = (event: MouseEvent) => {
    const remove = props.onRemove
    if (remove) showBotMenu(props.bot, remove, event)
  }
  return (
    <div class='player'>
      <span />
      <span />
      <svg class='icon rank bot' role='img'>
        <title>AI</title>
        <use href='#rank-bot' />
      </svg>
      <span />
      <SideIcon side={props.bot.status.side} />
      <span
        class='pname bot'
        classList={{ mine: props.onRemove !== undefined }}
        title={`${props.bot.ai} · ${props.bot.owner}`}
        onClick={menu}
        onContextMenu={menu}
      >
        {props.bot.name}
      </span>
      <Show when={props.onOptions}>
        <button
          class='bot-remove'
          title={`What ${props.bot.name} is told about itself`}
          aria-label={`Options for ${props.bot.name}`}
          onClick={() => props.onOptions?.()}
        >
          <Glyph id='act-pen' />
        </button>
      </Show>
      <Show when={props.onRemove}>
        <button
          class='bot-remove'
          title={`Remove ${props.bot.name}`}
          aria-label={`Remove ${props.bot.name}`}
          onClick={() => void props.onRemove?.()}
        >
          <Glyph id='act-trash' />
        </button>
      </Show>
    </div>
  )
}

/**
 * A seat the room's shape says exists, whose player the server has not
 * placed yet. Same height as a row, so the name lands without moving anything.
 */
export function EmptySeat() {
  return (
    <div class='player empty' aria-hidden='true'>
      <span />
      <span />
      <span />
      <span />
      <span />
      <span class='pname' />
    </div>
  )
}

/**
 * A member seated where we expect them, before the server has said so: the
 * name, flag and rank the lobby already knows, none of the seat's own marks.
 */
export function GuessedRow(props: {
  user: UserView
  me: boolean
  friend?: boolean
}) {
  return (
    <div class='player pending'>
      <span />
      <Flag country={props.user.country} />
      <RankIcon status={props.user.status} />
      <span class='skill none'>·</span>
      <span />
      <span
        class='pname'
        classList={{ me: props.me, friend: props.friend }}
        onClick={(event) => showPlayerMenu(props.user.name, event)}
        onContextMenu={(event) => showPlayerMenu(props.user.name, event)}
      >
        {props.user.name}
      </span>
    </div>
  )
}

/**
 * No status and no faction: Chobby hides both for spectators, and so do we.
 * `pending` is a member the server has not placed yet, listed here until it
 * does — most of them are about to move to a seat.
 */
export function SpectatorRow(props: {
  user: UserView
  me: boolean
  friend?: boolean
  boss?: boolean
  pending?: boolean
}) {
  return (
    <div class='spectator' classList={{ pending: props.pending }}>
      <Flag country={props.user.country} />
      <RankIcon status={props.user.status} />
      <span
        class='pname'
        classList={{
          me: props.me,
          friend: props.friend,
          bot: props.user.status.bot,
        }}
        onClick={(event) => showPlayerMenu(props.user.name, event)}
        onContextMenu={(event) => showPlayerMenu(props.user.name, event)}
      >
        {props.user.name}
      </span>
      <Marks status={props.user.status} boss={props.boss ?? false} />
    </div>
  )
}

function SkillCell(props: { skill: Skill | null }) {
  return (
    <Show when={props.skill} fallback={<span class='skill none'>·</span>}>
      {(skill) => (
        <span
          class={`skill tier${skillTier(skill())}`}
          title={skillTitle(skill())}
        >
          {skillText(skill())}
        </span>
      )}
    </Show>
  )
}
