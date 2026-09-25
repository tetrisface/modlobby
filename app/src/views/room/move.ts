import { bossesOf, freeTeam, isBoss } from '../../lib/roster'
import type { RoomModel } from './model'

/**
 * Moving somebody to a team, and giving an AI a bonus.
 *
 * One gesture, three mechanisms -- which is Chobby's arrangement too
 * (`api_user_handler.lua:1573`): your own seat is a battle status of your own,
 * an AI you added is an `UPDATEBOT`, and everybody else is a request to the
 * host in battle chat. Only the last of the three can be refused, and it is
 * refused often; see [`FORCE_REFUSED`].
 */

/** Who is being moved. */
export type Target =
	| { kind: 'me' }
	| { kind: 'player'; name: string }
	| {
			kind: 'bot'
			name: string
			/** Whether we added it. Anyone else's, the server ignores us for. */
			mine: boolean
			/** Kept as it is when only the team changes, and vice versa. */
			team: number
			handicap: number
			colour: number
	  }

/**
 * Whether the room arranges its own teams.
 *
 * While it does, SPADS declines to move anybody by hand -- and BAR's default
 * team preset ships `autoBalance` on, so this is the ordinary case rather than
 * an edge one (`spads.pl:8886`).
 *
 * A room that has not said counts as not balancing, and the move is attempted.
 * A BAR host broadcasts this to the whole room on every change and again as
 * each person joins, so silence means a host that is not BAR's -- rare, and not
 * worth refusing over. Letting SPADS answer for itself costs one line of battle
 * chat and is right whenever it is on.
 */
export function balancing(room: RoomModel): boolean {
	const mode = room.my()?.autoBalance
	return mode !== null && mode !== undefined && mode !== 'off'
}

/** A target with a name to put in a command: everybody except yourself. */
type Named = Exclude<Target, { kind: 'me' }>

/** Whether SPADS would take our word for it in this room. */
export function bossing(room: RoomModel): boolean {
	// A room with no host to ask is one we already run.
	if (!room.caps.spads) return true
	return isBoss(room.my()?.boss, room.me())
}

/**
 * Why a setting we change here would be refused, or `null` when it may be
 * taken -- directly or as a vote, which is the host's call. Worded for anyone
 * who plays, not for anyone who runs an autohost.
 *
 * Only what BAR's autohosts refuse for certain (`spads_config_bar`,
 * `commands_*.conf` `[bSet]`), since a false "no" keeps somebody out of a
 * room they could have changed:
 * - a boss is raised to level 100 by BarManager and sets directly, seated or
 *   not, game running or not; a moderator is 110 and does too;
 * - with a boss in the room everybody else is level 0;
 * - an event room takes it from level 100 only;
 * - otherwise a player may, between games, and a spectator may not.
 * Somebody given a level by name in `users.conf` may do more than this says;
 * the command stays copyable for them.
 */
export function setRefusal(room: RoomModel): string | null {
	if (!room.caps.spads) return null
	const me = room.me()
	if (me === null) return null
	const user = room.users()[me]
	const boss = room.my()?.boss
	if (isBoss(boss, me) || user?.status.moderator) return null
	if (bossesOf(boss).length > 0)
		return 'Only the room’s boss can change settings'
	if (room.my()?.preset === 'event')
		return 'Only a boss can change settings in an event'
	if (room.running() !== null)
		return 'Settings can be changed once the game is over'
	if (user?.battleStatus?.player !== true)
		return 'Join as a player to change settings'
	return null
}

/**
 * Why a list of mods we send from here would be refused, or `null` when it
 * may be taken, directly by a boss or as a vote. A mod host keeps BAR's vote
 * levels (`commands_bar_votes.conf` there): a seated player may call the
 * vote between games even while the room has a boss, and a spectator may
 * not.
 */
export function modRefusal(room: RoomModel): string | null {
	if (!room.caps.spads) return null
	const me = room.me()
	if (me === null) return null
	const user = room.users()[me]
	if (isBoss(room.my()?.boss, me) || user?.status.moderator) return null
	if (room.running() !== null) return 'Mods can change once the game is over'
	if (user?.battleStatus?.player !== true)
		return 'Join as a player to change mods'
	return null
}

/** Whether a setting we change here may be taken; see `setRefusal`. */
export function canSet(room: RoomModel): boolean {
	return setRefusal(room) === null
}

/** Whether this row can be moved at all, and so whether it can be dragged. */
export function movable(room: RoomModel, target: Target): boolean {
	if (target.kind === 'me') return true
	if (target.kind === 'bot' && target.mine) return true
	return bossing(room)
}

/**
 * `!force <name> team <n>`.
 *
 * SPADS counts teams from one and takes the one back off before it sends
 * `FORCEALLYNO` (`spads.pl:8996`), so the number here is one more than the
 * index everything else in this app uses. A bot's name is prefixed with `%`
 * so it is not searched for among the players (`spads.pl:8917`).
 */
export function forceTeam(target: Named, allyTeam: number): string {
	const who = target.kind === 'bot' ? `%${target.name}` : target.name
	return `!force ${who} team ${allyTeam + 1}`
}

/** `!force <name> bonus <percent>`; SPADS refuses anything over 100. */
export function forceBonus(target: Named, percent: number): string {
	const who = target.kind === 'bot' ? `%${target.name}` : target.name
	return `!force ${who} bonus ${Math.max(0, Math.min(100, Math.round(percent)))}`
}

/** The lowest engine team nobody holds, which is what a new seat needs. */
function ourTeam(room: RoomModel): number {
	const battle = room.battle()
	return battle ? freeTeam(battle, room.users(), room.me()) : 0
}

export async function moveTo(
	room: RoomModel,
	target: Target,
	allyTeam: number,
): Promise<void> {
	if (target.kind === 'me') {
		// Dropped back where we sit is not a move; taking the seat again would
		// only ask the room to note it.
		const me = room.me()
		const held = me === null ? undefined : room.users()[me]?.battleStatus
		if (held?.player && held.allyTeam === allyTeam) return
		await room.io.takeSeat(ourTeam(room), allyTeam)
		return
	}
	if (target.kind === 'bot' && target.mine) {
		// The message replaces the whole status, so everything not being changed
		// has to be sent again -- the bonus included.
		await room.io.updateBot(
			target.name,
			target.team,
			allyTeam,
			target.handicap,
			target.colour,
		)
		// A host keeps its own book of bonuses; the status we just sent does not
		// reliably reach it, so the bonus is said again the way Chobby does after
		// a move (`api_user_handler.lua:1585`).
		if (room.caps.spads && target.handicap > 0)
			await room.io.sayBattle(forceBonus(target, target.handicap))
		return
	}
	if (balancing(room)) {
		// Sending it anyway means the answer arrives in battle chat, where a
		// person mid-drag is not looking. Better to say it here, in the words
		// that name the fix.
		throw new Error(
			'the room is balancing its own teams; turn Auto balance off first',
		)
	}
	await room.io.sayBattle(forceTeam(target, allyTeam))
}

/**
 * A resource bonus, as a percentage.
 *
 * Not for your own seat: a bonus is something a host gives, and asking for one
 * for yourself is a request like any other -- by name, through the host.
 *
 * An AI of ours is asked of the host too, where there is one. The host writes
 * the start script from its own book of bonuses and answers in chat, where
 * the room can see what was set; Chobby does the same for its own AIs
 * (`api_user_handler.lua:1626`). Only a room with nobody to ask takes the
 * bonus straight into the AI's status.
 */
export async function setBonus(
	room: RoomModel,
	target: Named,
	percent: number,
	allyTeam: number,
): Promise<void> {
	const wanted = Math.max(0, Math.min(100, Math.round(percent)))
	if (target.kind === 'bot' && target.mine && !room.caps.spads) {
		await room.io.updateBot(
			target.name,
			target.team,
			allyTeam,
			wanted,
			target.colour,
		)
		return
	}
	await room.io.sayBattle(forceBonus(target, wanted))
}
