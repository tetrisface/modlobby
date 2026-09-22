/**
 * Console hooks for the dev build: emit what the runtime would, to see a room
 * state without staging it on a live server. Loaded by `main.tsx` in dev only.
 *
 *   dev.held()           a press on its way, held by the flood window 8 s
 *   dev.breath()         everyone else seated is ready: the room waits on you
 *                        (seated, between games; seats a stand-in if alone)
 *   dev.inGame('name')   a player's crossed swords; `false` takes them off
 *   dev.standIn(ally)    a made-up player joins, seated on that ally team
 *   dev.emit(delta)      any delta, to the room's server
 *
 * A fake lasts until the server's next word on the same thing. Reloading the
 * page drops them all: the runtime hands the page its real state again.
 */
import type { BattleStatusView } from './ipc/bindings/BattleStatusView'
import type { Delta } from './ipc/bindings/Delta'
import { applyDelta } from './store/apply'
import { myRoom, roomServer, roomSession } from './store/lobby'

/** Applies `deltas` as the runtime's own, filed under the room's server. */
export function emit(...deltas: Delta[]): void {
	const server = roomServer()
	if (server === undefined) throw new Error('not in a room')
	for (const delta of deltas) applyDelta(delta, server)
}

function mine() {
	const session = roomSession()
	const my = session?.myBattle
	if (!session || !my) throw new Error('not in a room')
	const me = session.me ?? ''
	return { my, me, seat: session.users[me]?.battleStatus ?? null }
}

const seatOf = (name: string, status: BattleStatusView): Delta => ({
	type: 'memberStatus',
	data: { name, status, teamColour: 0 },
})

/**
 * A press held by the flood window for `ms`: Ready flipped while seated, Play
 * while watching. Then the line "leaves" and stays on its way a moment, as a
 * real one waits for its answer, before the room is as it was.
 */
export function held(ms = 8000): void {
	const { my, seat } = mine()
	// Read now: the store merges each change into this same object.
	const back = {
		readyOnItsWay: my.readyOnItsWay,
		seatOnItsWay: my.seatOnItsWay,
	}
	const wish = seat?.player
		? { readyOnItsWay: !seat.ready }
		: { seatOnItsWay: { player: true, allyTeam: 0 } }
	emit({
		type: 'myBattle',
		data: { ...my, ...wish, heldUntilMs: Date.now() + ms },
	})
	const now = () => mine().my
	setTimeout(() => {
		emit({ type: 'myBattle', data: { ...now(), heldUntilMs: null } })
		setTimeout(
			() => emit({ type: 'myBattle', data: { ...now(), ...back } }),
			400,
		)
	}, ms)
}

/**
 * Everyone else seated ready and synced, and us not: the room waiting on us
 * alone. One of them is unready for a moment first, so a second call breathes
 * again; our own button stays asked, yellow, throughout.
 */
export function breath(): void {
	const { me, seat } = mine()
	if (!seat?.player) throw new Error('take a seat first')
	// `readiness` asks nothing while a game runs: its end unreadies everyone.
	if (roomSession()?.gameRunning)
		throw new Error('a game is running here; nothing is asked until it ends')
	const seatedOthers = () => {
		const users = roomSession()?.users ?? {}
		return (myRoom()?.members ?? []).flatMap((name) => {
			const status = users[name]?.battleStatus
			return name !== me && status?.player ? [{ name, status }] : []
		})
	}
	// Alone in the room, nobody could be waiting: seat somebody who can.
	if (seatedOthers().length === 0) standIn(seat.allyTeam)
	const others = seatedOthers()
	const everyone = (firstReady: boolean) =>
		others.map(({ name, status }, i) =>
			seatOf(name, { ...status, ready: firstReady || i > 0, sync: 'synced' }),
		)
	emit(seatOf(me, { ...seat, ready: false }), ...everyone(false))
	setTimeout(() => emit(...everyone(true)), 50)
}

/** The name `standIn` joins under. */
const STAND_IN = 'dev-stand-in'

/**
 * A made-up player who joins the room seated on `allyTeam`, for states that
 * need somebody else. Their row stays until the page is reloaded.
 */
export function standIn(allyTeam = 0): void {
	const id = mine().my.id
	emit(
		{
			type: 'userAdded',
			data: {
				name: STAND_IN,
				country: 'SE',
				userId: null,
				lobbyClient: 'modlobby',
				status: {
					inGame: false,
					away: false,
					rank: 0,
					moderator: false,
					bot: false,
				},
				battleStatus: null,
				battleId: id,
			},
		},
		{ type: 'member', data: { id, name: STAND_IN, joined: true } },
		seatOf(STAND_IN, {
			ready: false,
			team: 15,
			allyTeam,
			player: true,
			handicap: 0,
			sync: 'synced',
			side: 0,
		}),
	)
}

/** `name`'s in-game bit, ourselves by default. */
export function inGame(name?: string, on = true): void {
	const session = roomSession()
	const who = name ?? session?.me ?? ''
	const user = session?.users[who]
	if (!user) throw new Error(`nobody called ${who} here`)
	emit({
		type: 'userStatus',
		data: { name: who, status: { ...user.status, inGame: on } },
	})
}

Object.assign(window, { dev: { emit, held, breath, inGame, standIn } })
