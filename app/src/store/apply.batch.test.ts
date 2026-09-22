import { createEffect, createRoot } from 'solid-js'
import { describe, expect, test } from 'vitest'
import type { Delta } from '../ipc/bindings/Delta'
import type { UserView } from '../ipc/bindings/UserView'
import { applyMessage } from './apply'
import { lobby } from './lobby'

const user = (name: string): UserView => ({
	name,
	country: 'SE',
	userId: 1,
	lobbyClient: 'modlobby',
	status: { inGame: false, away: false, rank: 0, moderator: false, bot: false },
	battleStatus: null,
	battleId: 5,
})

const status = (name: string, team: number): Delta => ({
	type: 'memberStatus',
	data: {
		name,
		teamColour: 0,
		status: {
			ready: false,
			team,
			allyTeam: team % 2,
			player: true,
			handicap: 0,
			sync: 'synced',
			side: 0,
		},
	},
})

describe('applyMessage', () => {
	test('a deltas message is one render pass, however many lines it holds', () => {
		const names = ['a', 'b', 'c', 'd']
		applyMessage({
			type: 'snapshot',
			data: {
				servers: [
					{
						server: 'server4',
						phase: 'ready',
						retryIn: null,
						me: 'me',
						users: names.map(user),
						battles: [],
						myBattle: null,
						gameRunning: null,
						channels: [],
						friends: { friends: [], requests: [], ignored: [] },
					},
				],
				engine: { state: 'idle' },
				download: { state: 'idle' },
				paste: { state: 'idle' },
				skirmish: null,
				ways: {},
				content: null,
				contentCheck: { game: null, map: null },
			},
		})
		const runs: number[] = []
		const dispose = createRoot((dispose) => {
			createEffect(() => {
				runs.push(
					names.filter(
						(n) => lobby.servers.server4?.users[n]?.battleStatus?.player,
					).length,
				)
			})
			return dispose
		})
		applyMessage({
			type: 'deltas',
			data: { server: 'server4', deltas: names.map((n, i) => status(n, i)) },
		})
		dispose()
		// The initial run, then one for the whole burst — not one per line.
		expect(runs).toEqual([0, 4])
	})
})
