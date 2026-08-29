import { Show } from 'solid-js'
import type { BotView } from '../ipc/bindings/BotView'
import type { UserView } from '../ipc/bindings/UserView'
import { type Skill, skillText, skillTier, skillTitle } from '../lib/skill'
import { Flag, RankIcon, SideIcon, StatusIcon } from './icons'

/**
 * One player, in Chobby's column order: status, country, rank, skill, faction,
 * name. Nothing here is coloured by team — the engine assigns player colours at
 * game start, so a pre-game team hue would be a guess. The faction is the one
 * coloured mark, because that one is chosen and known.
 */
export function PlayerRow(props: {
  user: UserView
  skill: Skill | null
  me: boolean
}) {
  return (
    <Show when={props.user.battleStatus}>
      {(battle) => (
        <div class='player'>
          <StatusIcon status={props.user.status} battle={battle()} />
          <Flag country={props.user.country} />
          <RankIcon status={props.user.status} />
          <SkillCell skill={props.skill} />
          <SideIcon side={battle().side} />
          <span class='pname' classList={{ me: props.me }}>
            {props.user.name}
          </span>
        </div>
      )}
    </Show>
  )
}

/** An AI seat. It holds a team but has no lobby account behind it. */
export function BotRow(props: { bot: BotView }) {
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
      <span class='pname bot' title={`${props.bot.ai} · ${props.bot.owner}`}>
        {props.bot.name}
      </span>
    </div>
  )
}

/** No status and no faction: Chobby hides both for spectators, and so do we. */
export function SpectatorRow(props: { user: UserView; me: boolean }) {
  return (
    <div class='spectator'>
      <Flag country={props.user.country} />
      <RankIcon status={props.user.status} />
      <span
        class='pname'
        classList={{ me: props.me, bot: props.user.status.bot }}
      >
        {props.user.name}
      </span>
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
