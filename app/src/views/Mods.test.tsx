import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, beforeEach, describe, expect, test } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { forgetSet, modSets, rememberSet, setDraft } from '../store/mods'
import { Mods } from './Mods'
import { RoomProvider } from './room/model'
import {
	type Calls,
	fakeRoom,
	myBattle,
	recordingIo,
	status,
	user,
} from './room/fixture'

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

beforeEach(() => {
	setDraft(1, null)
	for (const set of modSets()) forgetSet(set.at)
})
afterEach(cleanup)

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
			<Mods />
		</RoomProvider>
	))
	const rows = () =>
		[...container.querySelectorAll('.mod-rows.loaded .mod-row')].map((row) => ({
			name: row.querySelector('.mod-name')?.textContent,
			source: row.querySelector('.mod-source')?.textContent ?? null,
			row,
		}))
	const offers = () => [...container.querySelectorAll('.mod-offers button')]
	const footer = () => container.querySelector('.mod-footer')
	const command = () => footer()?.querySelector('.mod-command')?.textContent
	const apply = () =>
		footer()?.querySelector<HTMLButtonElement>('button.primary') ?? null
	function paste(text: string) {
		const field = container.querySelector<HTMLInputElement>('.mod-add input')!
		fireEvent.input(field, { target: { value: text } })
		fireEvent.keyDown(field, { key: 'Enter' })
	}
	return { container, calls, rows, offers, footer, command, apply, paste }
}

describe('the mods pane', () => {
	test('shows what is loaded, where it comes from, and what the host offers', () => {
		const { rows, offers, footer } = pane({ loaded: [sphere] })
		expect(rows().map((row) => [row.name, row.source])).toEqual([
			['sphere spawner mod v1.0.0', 'dev/sphere @ 9108a17'],
		])
		expect(offers().map((chip) => chip.textContent)).toEqual([
			'sphere-spawner',
			'tiny maps v1',
		])
		expect(offers()[0]?.hasAttribute('disabled')).toBe(true)
		expect(footer()).toBeNull()
	})

	test('an offer clicked, a page pasted, and a row removed make one command', () => {
		const { rows, offers, command, paste, container } = pane({
			loaded: [sphere],
		})
		fireEvent.click(offers()[1]!)
		paste('https://github.com/dev/tanks/tree/wip')
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
			'tanks',
		])
		expect(command()).toBe(
			'!mutator set sphere-spawner, tiny maps v1, dev/tanks@wip',
		)
		expect(
			container.querySelector('.mod-footer .mod-summary')?.textContent,
		).toBe('2 added')
		fireEvent.click(rows()[0]!.row.querySelector('button.danger')!)
		expect(command()).toBe('!mutator set tiny maps v1, dev/tanks@wip')
	})

	test('what is neither an offer nor a repository is refused in place', () => {
		const { paste, container, footer } = pane({ loaded: [sphere] })
		paste('rogue archive')
		expect(container.querySelector('.mod-problem')?.textContent).toContain(
			'Not a name',
		)
		expect(footer()).toBeNull()
	})

	/** Lays the loaded rows out 40px apart, top to bottom, as jsdom will not. */
	function laidOut(container: HTMLElement) {
		const list = container.querySelector('.mod-rows.loaded')!
		for (const [index, row] of [...list.children].entries())
			row.getBoundingClientRect = () =>
				({ top: index * 40, height: 40, left: 0, width: 300 }) as DOMRect
	}

	test('a row is dragged into load order, the others making room', () => {
		const { rows, offers, command, container } = pane({ loaded: [sphere] })
		fireEvent.click(offers()[1]!)
		laidOut(container)
		fireEvent.pointerDown(rows()[1]!.row, { clientX: 10, clientY: 60 })
		fireEvent.pointerMove(window, { clientX: 10, clientY: 10 })
		expect(rows().map((row) => row.name)).toEqual([
			'tiny maps v1',
			'sphere spawner mod v1.0.0',
		])
		expect(document.querySelector('.drag-ghost')).not.toBeNull()
		expect(command()).toBe('!mutator set sphere-spawner, tiny maps v1')
		fireEvent.pointerUp(window, { clientX: 10, clientY: 10 })
		expect(document.querySelector('.drag-ghost')).toBeNull()
		expect(command()).toBe('!mutator set tiny maps v1, sphere-spawner')
	})

	test('a drag let go of, or a press on a control, moves nothing', () => {
		const { rows, offers, command, container } = pane({ loaded: [sphere] })
		fireEvent.click(offers()[1]!)
		laidOut(container)
		fireEvent.pointerDown(rows()[1]!.row, { clientX: 10, clientY: 60 })
		fireEvent.pointerMove(window, { clientX: 10, clientY: 10 })
		fireEvent.pointerCancel(window)
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
		fireEvent.pointerDown(rows()[1]!.row.querySelector('button')!, {
			clientX: 10,
			clientY: 60,
		})
		fireEvent.pointerMove(window, { clientX: 10, clientY: 10 })
		fireEvent.pointerUp(window, { clientX: 10, clientY: 10 })
		expect(command()).toBe('!mutator set sphere-spawner, tiny maps v1')
	})

	test('the summary is the legend of the row markers', () => {
		const { rows, offers, container } = pane({ loaded: [sphere] })
		fireEvent.click(offers()[1]!)
		fireEvent.click(rows()[0]!.row.querySelector('[aria-label^="Update"]')!)
		const parts = [...container.querySelectorAll('.mod-summary .mod-change')]
		expect(parts.map((part) => [part.className, part.textContent])).toEqual([
			['mod-change added', '1 added'],
			['mod-change moving', '1 to another commit'],
		])
		expect(rows()[0]?.row.getAttribute('title')).toContain(
			'Goes to another commit when the draft is applied',
		)
		expect(rows()[1]?.row.getAttribute('title')).toContain(
			'Added by this draft',
		)
	})

	test('a mod from GitHub is moved to the newest commit, or pointed elsewhere', () => {
		const { rows, command, container } = pane({ loaded: [sphere] })
		fireEvent.click(rows()[0]!.row.querySelector('[aria-label^="Update"]')!)
		expect(rows()[0]?.source).toBe('dev/sphere @ 9108a17 → newest')
		expect(rows()[0]?.row.classList.contains('moving')).toBe(true)
		expect(command()).toBe('!mutator set dev/sphere')
		fireEvent.click(rows()[0]!.row.querySelector('[aria-label^="Edit"]')!)
		const field = container.querySelector<HTMLInputElement>('.mod-source-edit')!
		fireEvent.input(field, { target: { value: 'dev/sphere@main' } })
		fireEvent.keyDown(field, { key: 'Enter' })
		expect(rows()[0]?.source).toBe('dev/sphere @ 9108a17 → newest of main')
		expect(command()).toBe('!mutator set dev/sphere@main')
	})

	test('a boss applies, a seated player votes, a spectator only drafts', () => {
		const boss = pane({ boss: 'me', loaded: [sphere] })
		fireEvent.click(boss.offers()[1]!)
		expect(boss.apply()?.textContent).toBe('Apply')
		fireEvent.click(boss.apply()!)
		expect(boss.calls).toEqual([
			['sayBattle', ['!mutator set sphere-spawner, tiny maps v1']],
		])
		cleanup()
		setDraft(1, null)
		const player = pane({ boss: 'someone', loaded: [sphere] })
		fireEvent.click(player.offers()[1]!)
		expect(player.apply()?.textContent).toBe('Vote to apply')
		expect(player.apply()?.disabled).toBe(false)
		cleanup()
		setDraft(1, null)
		const watcher = pane({ player: false, loaded: [sphere] })
		fireEvent.click(watcher.offers()[1]!)
		expect(watcher.apply()?.disabled).toBe(true)
		expect(watcher.apply()?.title).toBe('Join as a player to change mods')
		expect(watcher.command()).toBe('!mutator set sphere-spawner, tiny maps v1')
	})

	test('a draft the host has taken clears itself; discarded, it is gone', () => {
		const first = pane({ loaded: [sphere] })
		fireEvent.click(first.offers()[1]!)
		expect(first.footer()).not.toBeNull()
		fireEvent.click(first.footer()!.querySelector('button')!)
		expect(first.footer()).toBeNull()
		fireEvent.click(first.offers()[1]!)
		cleanup()
		const tiny: MutatorView = {
			...sphere,
			name: 'tiny maps v1',
			title: 'tiny maps v1',
			source: null,
		}
		const after = pane({ loaded: [sphere, tiny] })
		expect(after.footer()).toBeNull()
		expect(after.rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
	})

	test('a set a room loaded before comes back as the draft', () => {
		rememberSet([
			sphere,
			{ ...sphere, name: 'tiny maps v1', title: 'tiny maps v1', source: null },
		])
		const { container, command, rows } = pane({ loaded: [] })
		const set = container.querySelector('.mod-row.set')!
		expect(set.querySelector('.mod-name')?.textContent).toBe(
			'sphere spawner mod v1.0.0 + tiny maps v1',
		)
		fireEvent.click(set.querySelector('[aria-label^="Load"]')!)
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
		expect(command()).toBe('!mutator set sphere-spawner, tiny maps v1')
		fireEvent.click(set.querySelector('button.danger')!)
		expect(container.querySelector('.mod-row.set')).toBeNull()
	})
})
