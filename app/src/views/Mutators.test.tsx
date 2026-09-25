import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { Mutators } from './Mutators'
import { RoomProvider } from './room/model'
import {
	type Calls,
	fakeRoom,
	myBattle,
	recordingIo,
	status,
	user,
} from './room/fixture'

afterEach(cleanup)

const SHA = '9108a17078f79d09925edc305ec83bc06c3a7cb3'
const SPHERE = `github:dev/sphere@${SHA}`

const tags = {
	'game/mutatoroffer0': 'sphere-spawner',
	'game/mutatoroffer0source': SPHERE,
	'game/mutatoroffer1': 'tiny maps v1',
}

const sphere: MutatorView = {
	name: 'github-dev-sphere-9108a17078f7.sdd',
	title: 'sphere spawner mod v1.0.0',
	here: true,
	check: null,
	source: SPHERE,
	date: null,
}

function pane(options: {
	boss?: string
	player?: boolean
	loaded?: MutatorView[]
}) {
	const calls: Calls = []
	const room = fakeRoom({
		my: () => myBattle({ scriptTags: tags, boss: options.boss ?? null }),
		users: () => ({
			me: user('me', {
				battleStatus: status({ player: options.player ?? true }),
			}),
		}),
		check: () => ({ game: null, map: null, mutators: options.loaded ?? [] }),
		io: recordingIo(calls),
	})
	const { container } = render(() => (
		<RoomProvider value={room}>
			<Mutators />
		</RoomProvider>
	))
	const rows = () =>
		[...container.querySelectorAll('.mutator-row')].map((row) => ({
			name: row.querySelector('.mutator-name')?.textContent,
			button: row.querySelector('button'),
		}))
	return { container, calls, rows }
}

describe('the mutators pane', () => {
	test('lists what is loaded, then what the host offers that is not', () => {
		const { rows } = pane({ loaded: [sphere] })
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
	})

	test('a boss adds and removes by the name the host knows', () => {
		const { rows, calls } = pane({ boss: 'me', loaded: [sphere] })
		const [loaded, offer] = rows()
		expect(loaded?.button?.textContent).toBe('Remove')
		fireEvent.click(loaded!.button!)
		fireEvent.click(offer!.button!)
		expect(calls).toEqual([
			['sayBattle', ['!mutator remove sphere-spawner']],
			['sayBattle', ['!mutator add tiny maps v1']],
		])
	})

	test('a seated player is offered the vote, even with a boss in the room', () => {
		const { rows } = pane({ boss: 'someone', loaded: [sphere] })
		expect(rows().map((row) => row.button?.textContent)).toEqual([
			'Vote to remove',
			'Vote to add',
		])
		expect(rows().every((row) => !row.button?.disabled)).toBe(true)
	})

	test('a spectator sees the buttons greyed, and why', () => {
		const { rows, container } = pane({ player: false })
		expect(rows()[0]?.button?.disabled).toBe(true)
		expect(container.querySelector('.note')?.textContent).toBe(
			'Join as a player to change mutators',
		)
	})

	test('an update is offered only for mutators from GitHub', () => {
		const { container, calls } = pane({ boss: 'me', loaded: [sphere] })
		const update = [...container.querySelectorAll('.toolbar button')][0]
		fireEvent.click(update!)
		expect(calls).toEqual([['sayBattle', ['!mutator update']]])
		cleanup()
		const none = pane({ loaded: [] }).container
		expect(none.querySelectorAll('.toolbar button')).toHaveLength(0)
	})
})
