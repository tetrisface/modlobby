import { describe, expect, test } from 'vitest'
import {
	ALONE,
	SERVED,
	battle,
	fakeRoom,
	myBattle,
	recordingIo,
	status,
	user,
} from './fixture'
import type { Calls } from './fixture'
import {
	balancing,
	bossing,
	forceBonus,
	forceTeam,
	movable,
	moveTo,
	setBonus,
	setRefusal,
	type Target,
} from './move'

const ME: Target = { kind: 'me' }
const ALICE: Target = { kind: 'player', name: 'alice' }
const MY_BOT: Target = {
	kind: 'bot',
	name: 'BARb',
	mine: true,
	team: 3,
	handicap: 25,
	colour: 0x4b73f2,
}
const THEIR_BOT: Target = { ...MY_BOT, name: 'CHEATb', mine: false }

/** A room on the server, with `boss` whoever is named. */
function served(
	calls: Calls,
	boss: string | null = null,
	autoBalance: string | null = 'off',
) {
	return fakeRoom({
		caps: SERVED,
		battle: () => battle(),
		my: () => myBattle({ boss, autoBalance }),
		io: recordingIo(calls),
	})
}

describe('who may move whom', () => {
	test('a room with no host to ask is one we already run', () => {
		expect(bossing(fakeRoom({ caps: ALONE }))).toBe(true)
	})

	test('bossing a room is what lets you move other people', () => {
		expect(movable(served([], 'me'), ALICE)).toBe(true)
		expect(movable(served([], 'alice'), ALICE)).toBe(false)
		expect(movable(served([], null), ALICE)).toBe(false)
	})

	test('your own seat is always yours to move', () => {
		expect(movable(served([], 'alice'), ME)).toBe(true)
	})

	test('an AI you added is yours even when somebody else bosses the room', () => {
		expect(movable(served([], 'alice'), MY_BOT)).toBe(true)
	})

	test("and somebody else's AI is not, unless you boss it", () => {
		expect(movable(served([], 'alice'), THEIR_BOT)).toBe(false)
		expect(movable(served([], 'me'), THEIR_BOT)).toBe(true)
	})
})

describe('whether a setting we change would be taken', () => {
	/** A room on the server: seated or not, who bosses it, and what else is so. */
	function room(
		over: {
			seated?: boolean
			boss?: string | null
			moderator?: boolean
			running?: boolean
			preset?: string | null
		} = {},
	) {
		const me = user('me', {
			battleStatus: status({ player: over.seated ?? true }),
		})
		me.status.moderator = over.moderator ?? false
		return fakeRoom({
			caps: SERVED,
			users: () => ({ me }),
			my: () =>
				myBattle({ boss: over.boss ?? null, preset: over.preset ?? null }),
			running: () =>
				over.running ? { id: 1, ip: '1.2.3.4', port: 8452 } : null,
		})
	}

	test('a room of our own takes anything', () => {
		expect(setRefusal(fakeRoom({ caps: ALONE }))).toBeNull()
	})

	test('a player may between games; a spectator is told to join', () => {
		expect(setRefusal(room())).toBeNull()
		expect(setRefusal(room({ seated: false }))).toBe(
			'Join as a player to change settings',
		)
		expect(setRefusal(room({ running: true }))).toBe(
			'Settings can be changed once the game is over',
		)
	})

	test('a boss sets from anywhere, and is still one of two', () => {
		// BarManager raises every boss to level 100, which bSet needs for itself.
		expect(setRefusal(room({ seated: false, boss: 'me' }))).toBeNull()
		expect(setRefusal(room({ running: true, boss: 'me' }))).toBeNull()
		expect(setRefusal(room({ seated: false, boss: 'alice, me' }))).toBeNull()
	})

	test('with a boss in the room, nobody else may', () => {
		expect(setRefusal(room({ boss: 'alice' }))).toBe(
			'Only the room’s boss can change settings',
		)
	})

	test('a moderator may, even watching, even with a boss', () => {
		expect(
			setRefusal(room({ seated: false, boss: 'alice', moderator: true })),
		).toBeNull()
	})

	test('an event room takes it from its boss only', () => {
		expect(setRefusal(room({ preset: 'event' }))).toBe(
			'Only a boss can change settings in an event',
		)
		expect(setRefusal(room({ preset: 'event', boss: 'me' }))).toBeNull()
	})
})

describe('the command SPADS is sent', () => {
	test('teams are counted from one, because SPADS takes one back off', () => {
		expect(forceTeam(ALICE, 0)).toBe('!force alice team 1')
		expect(forceTeam(ALICE, 2)).toBe('!force alice team 3')
	})

	test('a bot is marked so it is not searched for among the players', () => {
		expect(forceTeam(THEIR_BOT, 1)).toBe('!force %CHEATb team 2')
	})

	test('a bonus is a percentage, and SPADS refuses more than a hundred', () => {
		expect(forceBonus(ALICE, 50)).toBe('!force alice bonus 50')
		expect(forceBonus(ALICE, 400)).toBe('!force alice bonus 100')
		expect(forceBonus(ALICE, -5)).toBe('!force alice bonus 0')
	})
})

describe('moving somebody', () => {
	test('your own seat is a status of your own, not a request', async () => {
		const calls: Calls = []
		await moveTo(served(calls, 'alice'), ME, 1)
		expect(calls).toEqual([['takeSeat', [0, 1]]])
	})

	test('dropped back on the team you sit on, nothing is sent', async () => {
		const calls: Calls = []
		// The fixture seats us on the first team.
		await moveTo(served(calls, 'alice'), ME, 0)
		expect(calls).toEqual([])
	})

	test('an AI you added moves without asking the host', async () => {
		const calls: Calls = []
		await moveTo(served(calls, 'alice'), MY_BOT, 2)
		// Its team, bonus and colour go out again: the message replaces the whole
		// status, so anything not resent is lost. The host keeps its own book of
		// bonuses, so the bonus is said to it once more, as Chobby does.
		expect(calls).toEqual([
			['updateBot', ['BARb', 3, 2, 25, 0x4b73f2]],
			['sayBattle', ['!force %BARb bonus 25']],
		])
	})

	test('an AI with no bonus has none to repeat after the move', async () => {
		const calls: Calls = []
		await moveTo(served(calls, 'alice'), { ...MY_BOT, handicap: 0 }, 2)
		expect(calls).toEqual([['updateBot', ['BARb', 3, 2, 0, 0x4b73f2]]])
	})

	test('anybody else is a request to the host in chat', async () => {
		const calls: Calls = []
		await moveTo(served(calls, 'me'), ALICE, 1)
		expect(calls).toEqual([['sayBattle', ['!force alice team 2']]])
	})

	test("somebody else's AI too, since the server would ignore us", async () => {
		const calls: Calls = []
		await moveTo(served(calls, 'me'), THEIR_BOT, 0)
		expect(calls).toEqual([['sayBattle', ['!force %CHEATb team 1']]])
	})
})

describe('a bonus', () => {
	test('is asked of the host for an AI of ours, where there is one', async () => {
		const calls: Calls = []
		await setBonus(served(calls, null), MY_BOT, 50, 1)
		expect(calls).toEqual([['sayBattle', ['!force %BARb bonus 50']]])
	})

	test('rides the status where there is no host to ask', async () => {
		const calls: Calls = []
		const alone = fakeRoom({
			caps: ALONE,
			battle: () => battle(),
			io: recordingIo(calls),
		})
		await setBonus(alone, MY_BOT, 50, 1)
		expect(calls).toEqual([['updateBot', ['BARb', 3, 1, 50, 0x4b73f2]]])
	})

	test('and is a chat command for anyone else', async () => {
		const calls: Calls = []
		await setBonus(served(calls, 'me'), ALICE, 50, 1)
		expect(calls).toEqual([['sayBattle', ['!force alice bonus 50']]])
	})
})

describe('a room that arranges its own teams', () => {
	test("BAR's default is to, which SPADS then refuses to override", () => {
		expect(balancing(served([], 'me', 'advanced'))).toBe(true)
		expect(balancing(served([], 'me', 'on'))).toBe(true)
		expect(balancing(served([], 'me', 'off'))).toBe(false)
	})

	test('a room that has not said is tried anyway', () => {
		// Silence means a host that does not report its settings at all, rather
		// than one that is not balancing. SPADS answers for itself, in chat,
		// when it does object.
		expect(balancing(served([], 'me', null))).toBe(false)
	})

	test('moving a player says why rather than being quietly declined', async () => {
		const calls: Calls = []
		await expect(
			moveTo(served(calls, 'me', 'advanced'), ALICE, 1),
		).rejects.toThrow(/balancing its own teams/)
		expect(calls).toEqual([])
	})

	test('but an AI of ours does not go through the host to move', async () => {
		const calls: Calls = []
		await moveTo(served(calls, 'me', 'advanced'), MY_BOT, 1)
		// Only the bonus is said to it, which SPADS takes whatever the balance
		// setting: `hForce` refuses `team` and `id` under auto balance, not `bonus`.
		expect(calls).toEqual([
			['updateBot', ['BARb', 3, 1, 25, 0x4b73f2]],
			['sayBattle', ['!force %BARb bonus 25']],
		])
	})
})
