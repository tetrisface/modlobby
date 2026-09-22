import { describe, expect, test } from 'vitest'
import type { BattleStatusView } from '../../ipc/bindings/BattleStatusView'
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

const looks = (room: ReturnType<typeof fakeRoom>) => {
	const p = posture(room)
	return [p.watch.look, p.play.look, p.ready.look]
}

describe('the posture control', () => {
	test('watching holds Spectate and offers Play as the next step', () => {
		expect(looks(room({ player: false }))).toEqual(['held', 'next', 'plain'])
	})

	test('a Ready pressed while watching is on its way before the seat is', () => {
		const p = posture(room({ player: false }, {}, {}, { readyOnItsWay: true }))
		expect(p.ready.pending).toBe(true)
	})

	test('queued holds Play with the place, and Ready arms for the seat', () => {
		const queued = (my: Partial<MyBattleView>) =>
			posture(
				room(
					{ player: false },
					{},
					{
						battle: () =>
							battle({ members: ['me', 'alice'], queue: ['alice', 'me'] }),
					},
					my,
				),
			)
		expect(queued({}).play).toMatchObject({
			look: 'held',
			note: 'queued 2 of 2',
		})
		expect(queued({}).ready.look).toBe('plain')
		expect(queued({ preReady: true }).ready).toMatchObject({
			look: 'armed',
			note: 'when seated',
		})
	})

	test('seated and not ready, Ready is the one thing asked', () => {
		expect(looks(room({}))).toEqual(['plain', 'held', 'asked'])
	})

	test('and it is waited on once everyone else is ready', () => {
		const p = posture(room({}, { alice: { ready: true } }))
		expect(p.ready.look).toBe('asked')
		expect(p.ready.waiting).toBe(true)
	})

	test('ready holds Play and Ready', () => {
		expect(looks(room({ ready: true }))).toEqual(['plain', 'held', 'held'])
	})

	test('a ready asked for shows held at once, as pending', () => {
		const p = posture(room({}, {}, {}, { readyOnItsWay: true }))
		expect(p.ready).toMatchObject({ look: 'held', pending: true })
	})

	test('a seat asked for while watching shows Play at once, on its way', () => {
		const p = posture(
			room(
				{ player: false },
				{},
				{},
				{
					seatOnItsWay: { player: true, allyTeam: 0 },
				},
			),
		)
		expect(p.watch.look).toBe('plain')
		expect(p.play).toMatchObject({ look: 'held', pending: true })
	})

	test('a stand-up asked for shows Spectate at once, on its way', () => {
		const p = posture(
			room({}, {}, {}, { seatOnItsWay: { player: false, allyTeam: 0 } }),
		)
		expect(p.watch).toMatchObject({ look: 'held', pending: true })
		expect(p.play.look).toBe('next')
	})

	test('the hold by the flood window rides along', () => {
		expect(posture(room({})).heldUntil).toBeNull()
		expect(posture(room({}, {}, {}, { heldUntilMs: 1234 })).heldUntil).toBe(
			1234,
		)
	})

	test('during a game the ready offered is one for the next', () => {
		const running = (my: Partial<MyBattleView>) =>
			posture(
				room(
					{ ready: true },
					{},
					{ running: () => ({ id: 1, ip: '', port: 0 }) },
					my,
				),
			)
		expect(running({}).play).toMatchObject({ look: 'held', note: 'next game' })
		expect(running({}).ready).toMatchObject({
			look: 'plain',
			note: 'next game',
		})
		expect(running({ preReady: true }).ready.look).toBe('armed')
	})
})
