import { Show } from 'solid-js'
import type { BotView } from '../ipc/bindings/BotView'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { UserView } from '../ipc/bindings/UserView'
import { type Skill, skillText, skillTier, skillTitle } from '../lib/skill'
import { Flag, Glyph, Marks, RankIcon, SideIcon, StatusIcon } from './icons'
import { rowGesture } from '../lib/drag'
import { type Moves, showBotMenu, showPlayerMenu } from './PlayerMenu'

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
  /** Where this row may be sent. Absent outside a room, or where it may not. */
  moves?: Moves
}) {
  const menu = (event: MouseEvent) =>
    showPlayerMenu(props.user.name, event, props.moves)
  const press = rowGesture({
    canMove: () => props.moves !== undefined,
    onMove: (ally) => void props.moves?.to(ally),
    onMenu: menu,
  })
  return (
    <Show when={props.user.battleStatus}>
      {(battle) => (
        <div
          class='player'
          classList={{ movable: props.moves !== undefined }}
          onPointerDown={press}
        >
          <StatusIcon
            status={props.user.status}
            battle={battle()}
            download={props.download}
          />
          <Flag country={props.user.country} />
          <RankIcon status={props.user.status} />
          <SkillCell skill={props.skill} />
          <SideIcon side={battle().side} />
          {/* The press is handled by the row, so that a drag off the name
              is the same gesture as a drag off anywhere else in it. */}
          <span
            class='pname'
            classList={{ me: props.me, friend: props.friend }}
            onContextMenu={menu}
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
  /** Where this AI may be sent, and what bonus it may be given. */
  moves?: Moves
  /** Another of the same, on the same team. Ours to add, so ours to copy. */
  onClone?: () => Promise<void>
}) {
  const menu = (event: MouseEvent) => {
    const remove = props.onRemove
    if (remove) showBotMenu(props.bot, remove, event, props.moves)
  }
  const press = rowGesture({
    canMove: () => props.moves !== undefined,
    onMove: (ally) => void props.moves?.to(ally),
    onMenu: menu,
  })
  /**
   * Scavengers and Raptors are game modes rather than opponents: the room
   * holds one, so there is no second to copy. Known by name, since a row is
   * not told what the game's `luaai.lua` declares.
   */
  const gameMode = () => /raptor|scav/i.test(props.bot.ai)
  const showSideIcon = () =>
    !gameMode() && !props.bot.ai.toLowerCase().includes('barb')
  const botRowClass = () => ({
    player: true,
    'bot-row': true,
    'bot-row-no-side': !showSideIcon(),
    movable: props.moves !== undefined,
  })

  return (
    <div classList={botRowClass()} onPointerDown={press}>
      <svg class='icon rank bot' role='img'>
        <title>AI</title>
        <use href='#rank-bot' />
      </svg>
      <Show when={showSideIcon()}>
        <SideIcon side={props.bot.status.side} />
      </Show>
      <span
        class='pname bot'
        classList={{ mine: props.onRemove !== undefined }}
        title={`${props.bot.ai} · ${props.bot.owner}`}
        onContextMenu={menu}
      >
        {props.bot.name}
      </span>
      {/* An AI has no rating, rank or country, so the columns those would sit
          in are the AI's to use: a bonus reads there rather than pushing the
          name into an ellipsis. */}
      <Show when={props.bot.status.handicap > 0}>
        <span class='bot-bonus' title='Resource bonus'>
          +{props.bot.status.handicap}%
        </span>
      </Show>
      <Show when={props.onClone && !gameMode()}>
        <button
          class='row-act bot-clone'
          title={`Another ${props.bot.ai} on this team`}
          aria-label={`Add another ${props.bot.ai}`}
          onClick={() => void props.onClone?.()}
        >
          <Glyph id='act-copy' />
        </button>
      </Show>
      <Show when={props.onOptions}>
        <button
          class='row-act bot-edit'
          title={`What ${props.bot.name} is told about itself`}
          aria-label={`Options for ${props.bot.name}`}
          onClick={() => props.onOptions?.()}
        >
          <Glyph id='act-pen' />
        </button>
      </Show>
      <Show when={props.onRemove}>
        <button
          class='row-act bot-remove'
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
 * Somebody watching: in the join queue or simply spectating. No status and
 * no faction, as Chobby hides both for spectators; the skill stays, since
 * who is waiting to play is a question about strength as much as names. A
 * host that sent no skill leaves the cell empty rather than marked.
 *
 * `pending` is a member the server has not placed yet, listed here until it
 * does — most of them are about to move to a seat. `place` is a position in
 * the join queue, counted from one, drawn as Chobby draws it: `1.` before
 * the flag.
 */
export function WatcherRow(props: {
  user: UserView
  skill: Skill | null
  me: boolean
  friend?: boolean
  boss?: boolean
  pending?: boolean
  place?: number
}) {
  return (
    <div
      class='watcher'
      classList={{ pending: props.pending, queued: props.place !== undefined }}
    >
      <Show when={props.place}>
        {(place) => <span class='place'>{place()}.</span>}
      </Show>
      <Flag country={props.user.country} />
      <RankIcon status={props.user.status} />
      <SkillCell skill={props.skill} absent='' />
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

/** `absent` is what stands in for a skill nobody sent; a dot by default. */
function SkillCell(props: { skill: Skill | null; absent?: string }) {
  return (
    <Show
      when={props.skill}
      fallback={<span class='skill none'>{props.absent ?? '·'}</span>}
    >
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
