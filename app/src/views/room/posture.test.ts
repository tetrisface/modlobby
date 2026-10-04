import { describe, expect, test } from 'vitest'
import type { BattleStatusView } from '../../ipc/bindings/BattleStatusView'
import type { IntentView } from '../../ipc/bindings/IntentView'
import type { MyBattleView } from '../../ipc/bindings/MyBattleView'
import { battle, fakeRoom, myBattle, status, user } from './fixture'
import { posture } from './posture'

/** A served room of `me` and whoever else is named, with their statuses. */
function room(
	mine: Partial<BattleStatusView>,
	others: Record<string, Partial<BattleStatusView>> = {},
	over: Parameters<typeof fakeRoom>[0] = {},
	my: Partial<MyBattleView> = {},
) {
	const users = Object.fromEntries(
		Object.entries({ me: mine, ...others }).map(([name, seat]) => [
			name,
			user(name, { battleStatus: status(seat) }),
		]),
	)
	return fakeRoom({
		battle: () => battle({ members: Object.keys(users) }),
		users: () => users,
		my: () => myBattle(my),
		...over,
	})
}

const running = () => ({
	id: 1,
	ip: '',
	port: 0,
	added: true,
	playingWith: null,
	vacated: [],
})

const queued = () =>
	battle({ members: ['me', 'alice'], queue: ['alice', 'me'] })

/** The posture of `me` asking for `intent`, with the server's word in `mine`. */
const at = (
	intent: IntentView,
	mine: Partial<BattleStatusView> = {},
	over: Parameters<typeof fakeRoom>[0] = {},
	my: Partial<MyBattleView> = {},
) => posture(room(mine, {}, over, { intent, ...my }))

const looks = (p: ReturnType<typeof posture>) => [
	p.watch.look,
	p.play.look,
	p.ready.look,
]

describe('the posture control', () => {
	test('watching holds Spectate and offers Play as the next step', () => {
		const p = at('spectate', { player: false })
		expect(p.stance).toBe('spectate')
		expect(looks(p)).toEqual(['held', 'next', 'plain'])
		expect(p.ready.note).toBeNull()
	})

	test('watching a running game, Ready is offered for the next one', () => {
		const p = at('spectate', { player: false }, { running })
		expect(p.ready).toMatchObject({ look: 'plain', note: 'next game' })
	})

	test('queued holds Play with the place; a ready given waits for the seat', () => {
		const p = at('play', { player: false }, { battle: queued })
		expect(p.stance).toBe('queuing')
		expect(p.play).toMatchObject({ look: 'held', note: 'queued 2 of 2' })
		expect(p.ready.look).toBe('plain')
		for (const intent of ['ready', 'readyNext'] as const)
			expect(
				at(intent, { player: false }, { battle: queued }).ready,
			).toMatchObject({ look: 'held', note: 'when seated' })
	})

	test('seated between games and not ready, Ready is the one thing asked', () => {
		const p = at('play')
		expect(p.stance).toBe('play')
		expect(looks(p)).toEqual(['plain', 'held', 'asked'])
	})

	test('and it is waited on once everyone else is ready', () => {
		const p = posture(
			room({}, { alice: { ready: true } }, {}, { intent: 'play' }),
		)
		expect(p.ready.look).toBe('asked')
		expect(p.ready.waiting).toBe(true)
	})

	test('ready holds Play and Ready', () => {
		const p = at('ready', { ready: true })
		expect(p.stance).toBe('ready')
		expect(looks(p)).toEqual(['plain', 'held', 'held'])
	})

	test('a ready asked for shows held at once, as pending', () => {
		const p = at('ready', {}, {}, { readyOnItsWay: true })
		expect(p.ready).toMatchObject({ look: 'held', pending: true })
	})

	test('a seat asked for while watching shows Play at once, on its way', () => {
		const p = at(
			'play',
			{ player: false },
			{},
			{ seatOnItsWay: { player: true, allyTeam: 0 } },
		)
		expect(p.watch.look).toBe('plain')
		expect(p.play).toMatchObject({ look: 'held', pending: true })
	})

	test('a seat wanted that the room has not shown yet stays Play, pending', () => {
		// The room still to say whether its queue took us.
		const p = at('ready', { player: false })
		expect(p.play).toMatchObject({ look: 'held', pending: true })
		expect(p.ready.look).toBe('held')
	})

	test('a stand-up asked for shows Spectate at once, on its way', () => {
		const p = at(
			'spectate',
			{},
			{},
			{
				seatOnItsWay: { player: false, allyTeam: 0 },
			},
		)
		expect(p.watch).toMatchObject({ look: 'held', pending: true })
		expect(p.play.look).toBe('next')
	})

	test('the hold by the flood window rides along', () => {
		expect(at('play').heldUntil).toBeNull()
		expect(at('play', {}, {}, { heldUntilMs: 1234 }).heldUntil).toBe(1234)
	})

	test('during a game the seat is for the next one, and so is a ready for it', () => {
		const playing = at('play', {}, { running })
		expect(playing.stance).toBe('playNext')
		expect(playing.play).toMatchObject({ look: 'held', note: 'next game' })
		expect(playing.ready).toMatchObject({ look: 'plain', note: 'next game' })

		// A plain ready ends with the game it was given in.
		expect(at('ready', { ready: true }, { running }).stance).toBe('playNext')

		const ready = at('readyNext', { ready: true }, { running })
		expect(ready.stance).toBe('readyNext')
		expect(ready.ready).toMatchObject({ look: 'held', note: 'next game' })
	})

	test('ready implies playing: Ready is never held without Play', () => {
		const intents = ['spectate', 'play', 'ready', 'readyNext'] as const
		for (const intent of intents)
			for (const player of [true, false])
				for (const inQueue of [true, false])
					for (const game of [true, false]) {
						const p = at(
							intent,
							{ player },
							{
								...(inQueue ? { battle: queued } : {}),
								...(game ? { running } : {}),
							},
						)
						if (p.ready.look === 'held')
							expect(
								p.play.look,
								`${intent} player=${player} queued=${inQueue} game=${game}`,
							).toBe('held')
					}
	})
})
