import { describe, expect, test } from 'vitest'
import type { BattleStatusView } from '../../ipc/bindings/BattleStatusView'
import { ALONE, battle, fakeRoom, status, user } from './fixture'
import { readiness } from './readiness'

/** A served room of `me` plus whoever else is named, each with their status. */
function room(
	mine: Partial<BattleStatusView>,
	others: Record<string, Partial<BattleStatusView>> = {},
	over: Parameters<typeof fakeRoom>[0] = {},
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
		...over,
	})
}

describe('how much readying up asks of us', () => {
	test('nothing while watching, in a skirmish, or while a game runs', () => {
		expect(readiness(room({ player: false }))).toBe('none')
		expect(readiness(room({}, {}, { caps: ALONE }))).toBe('none')
		expect(
			readiness(
				room(
					{},
					{},
					{ running: () => ({ id: 1, ip: '', port: 0, added: true }) },
				),
			),
		).toBe('none')
	})

	test('nothing more once ready', () => {
		expect(readiness(room({ ready: true }, { alice: {} }))).toBe('settled')
	})

	test('asked while anyone else is still getting there', () => {
		// Not ready, and ready but still downloading: SPADS refuses both.
		expect(readiness(room({}, { alice: { ready: true }, bob: {} }))).toBe(
			'asked',
		)
		expect(
			readiness(room({}, { alice: { ready: true, sync: 'unsynced' } })),
		).toBe('asked')
	})

	test('waited on once every other player is ready and synced', () => {
		expect(
			readiness(
				room(
					{},
					{
						alice: { ready: true },
						bob: { ready: true },
						// Watching, so not somebody the room waits on.
						carol: { player: false },
					},
				),
			),
		).toBe('waiting')
	})

	test('alone against AIs is asked, not waited on', () => {
		expect(readiness(room({}))).toBe('asked')
	})
})
