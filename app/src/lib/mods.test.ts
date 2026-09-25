import { describe, expect, test } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import type { Offer } from './mutators'
import {
	type Pick,
	addPick,
	adopt,
	changeOf,
	command,
	editPick,
	movePick,
	parseGithub,
	picksOf,
	readSets,
	remember,
	sameList,
	sourceLinks,
	summary,
	updatePick,
	updatedTo,
	writeSets,
} from './mods'

const SHA = '9108a17078f79d09925edc305ec83bc06c3a7cb3'
const OTHER = 'b'.repeat(40)
const SPHERE_SOURCE = `github:dev/sphere@${SHA}`

const offered: Offer[] = [
	{
		name: 'sphere-spawner',
		source: SPHERE_SOURCE,
		date: '2025-10-05T13:32:07Z',
	},
	{ name: 'tiny maps v1', source: null, date: null },
]

const sphere: MutatorView = {
	name: 'github-dev-sphere-9108a17078f7.sdd',
	title: 'sphere spawner mod v1.0.0',
	description: null,
	here: true,
	check: null,
	source: SPHERE_SOURCE,
	date: '2025-10-05T13:32:07Z',
}
const tiny: MutatorView = {
	name: 'tiny maps v1',
	title: 'tiny maps v1',
	description: null,
	here: true,
	check: null,
	source: null,
	date: null,
}

const room = () => picksOf([sphere, tiny], offered)

describe('what somebody pastes', () => {
	test('is a repository in the forms GitHub and git print', () => {
		for (const [text, repo, ref] of [
			['dev/sphere', 'dev/sphere', null],
			['dev/sphere@main', 'dev/sphere', 'main'],
			[`dev/sphere@${SHA}`, 'dev/sphere', SHA],
			['https://github.com/dev/sphere', 'dev/sphere', null],
			['https://github.com/dev/sphere/', 'dev/sphere', null],
			['https://github.com/dev/sphere.git', 'dev/sphere', null],
			['github.com/dev/sphere/tree/feature/x', 'dev/sphere', 'feature/x'],
			[' https://www.github.com/dev/sphere/tree/main ', 'dev/sphere', 'main'],
		] as const) {
			expect(parseGithub(text), text).toEqual({ repo, ref })
		}
	})

	test('or nothing', () => {
		for (const text of [
			'tiny maps v1',
			'dev',
			'dev/sphere/extra',
			'dev/sphere@../up',
			'https://gitlab.com/dev/sphere',
		])
			expect(parseGithub(text), text).toBeNull()
	})
})

describe('the room as picks', () => {
	test('names each mod as its host knows it, and keeps where it came from', () => {
		expect(room()).toEqual([
			{
				ref: 'sphere-spawner',
				label: 'sphere spawner mod v1.0.0',
				repo: 'dev/sphere',
				source: SPHERE_SOURCE,
				date: '2025-10-05T13:32:07Z',
				description: null,
			},
			{
				ref: 'tiny maps v1',
				label: 'tiny maps v1',
				repo: null,
				source: null,
				date: null,
				description: null,
			},
		])
	})

	test('one the host does not offer is asked for by its commit', () => {
		const stranger = { ...sphere, source: `github:dev/tanks@${OTHER}` }
		expect(picksOf([stranger], offered)[0]?.ref).toBe(`dev/tanks@${OTHER}`)
	})
})

describe('editing a draft', () => {
	test('adds an offer by name, however it is typed', () => {
		const next = addPick([], 'Tiny  Maps V1', offered)
		expect('picks' in next && next.picks[0]?.ref).toBe('tiny maps v1')
	})

	test('adds a repository from a pasted page, labelled by its name', () => {
		const next = addPick([], 'https://github.com/dev/tanks/tree/wip', offered)
		expect('picks' in next && next.picks[0]).toEqual({
			ref: 'dev/tanks@wip',
			label: 'tanks',
			repo: 'dev/tanks',
			source: null,
			date: null,
		})
	})

	test('refuses what is neither, a repeat, and an eleventh', () => {
		expect(addPick([], 'rogue archive', offered)).toEqual({
			problem: expect.stringContaining('Not a name'),
		})
		expect(addPick(room(), 'dev/sphere@main', offered)).toEqual({
			problem: 'sphere spawner mod v1.0.0 is already in the list',
		})
		expect(addPick(room(), 'TINY MAPS v1', offered)).toEqual({
			problem: 'tiny maps v1 is already in the list',
		})
		const full = Array.from({ length: 10 }, (_, i) =>
			'picks' in addPick([], `dev/m${i}`, offered) ? `dev/m${i}` : '',
		).reduce<Pick[]>((picks, ref) => {
			const next = addPick(picks, ref, offered)
			return 'picks' in next ? next.picks : picks
		}, [])
		expect(addPick(full, 'dev/more', offered)).toEqual({
			problem: 'A room loads at most 10 mods',
		})
	})

	test('moves, updates and edits', () => {
		const [first, second] = room()
		expect(movePick(room(), 0, 1).map((pick) => pick.ref)).toEqual([
			second!.ref,
			first!.ref,
		])
		expect(updatePick(first!).ref).toBe('dev/sphere')
		expect(updatePick(second!)).toBe(second)
		expect(editPick(first!, 'dev/sphere@main')).toEqual({
			...first,
			ref: 'dev/sphere@main',
		})
		expect(editPick(first!, 'github.com/dev/tanks')).toEqual({
			ref: 'dev/tanks',
			label: 'tanks',
			repo: 'dev/tanks',
			source: null,
			date: null,
			description: null,
		})
		expect(editPick(first!, 'nonsense')).toBeNull()
	})
})

describe('what a draft says', () => {
	test('where a GitHub mod stands, and where it is going, as links', () => {
		const [first, second] = room()
		const links = (pick: Pick) => {
			const found = sourceLinks(pick)
			return (
				found &&
				[found.at, found.to].map((link) => link && [link.text, link.url])
			)
		}
		const AT = ['dev/sphere', `https://github.com/dev/sphere/tree/${SHA}`]
		expect(links(first!)).toEqual([AT, null])
		expect(links(updatePick(first!))).toEqual([
			AT,
			['newest', 'https://github.com/dev/sphere/commits'],
		])
		expect(links(editPick(first!, 'dev/sphere@main')!)).toEqual([
			AT,
			['newest of main', 'https://github.com/dev/sphere/commits/main'],
		])
		expect(links(editPick(first!, `dev/sphere@${OTHER}`)!)).toEqual([
			AT,
			['another commit', `https://github.com/dev/sphere/tree/${OTHER}`],
		])
		expect(links(editPick(first!, 'dev/tanks')!)).toEqual([
			['dev/tanks', 'https://github.com/dev/tanks'],
			['newest', 'https://github.com/dev/tanks/commits'],
		])
		expect(links(second!)).toBeNull()
	})

	test('the commit is in the tooltip, never in the text', () => {
		const [first] = room()
		const found = sourceLinks(updatePick(first!))!
		expect(found.at.tip).toContain(SHA)
		expect(`${found.at.text} ${found.to?.text}`).not.toMatch(/[0-9a-f]{7}/)
	})

	test('an update moves only what GitHub has a newer commit of', () => {
		const [first, second] = room()
		const newer = { sha: OTHER, date: '2025-11-01T00:00:00Z' }
		expect(updatedTo(first!, { sha: SHA, date: null }, room())).toEqual({
			pick: first,
			already: true,
		})
		expect(
			updatedTo(updatePick(first!), { sha: SHA, date: null }, room()),
		).toEqual({ pick: first, already: true })
		expect(updatedTo(first!, newer, room())).toEqual({
			pick: { ...first, ref: 'dev/sphere', newest: newer.date },
			already: false,
		})
		expect(sourceLinks(updatedTo(first!, newer, room()).pick)?.to?.date).toBe(
			newer.date,
		)
		expect(updatedTo(second!, newer, room())).toEqual({
			pick: second,
			already: false,
		})
	})

	test('how each pick differs from the room', () => {
		const [first] = room()
		expect(changeOf(first!, room())).toBe('same')
		expect(changeOf(updatePick(first!), room())).toBe('moving')
		expect(changeOf(editPick(first!, 'dev/tanks')!, room())).toBe('added')
	})

	test('the command, and a summary', () => {
		expect(command(room())).toBe('!mutator set sphere-spawner, tiny maps v1')
		expect(command([])).toBe('!mutator clear')
		const [first, second] = room()
		expect(summary([second!, first!], room())).toEqual([
			{ change: 'reordered', words: 'reordered' },
		])
		expect(summary([updatePick(first!)], room())).toEqual([
			{ change: 'removed', words: '1 removed' },
			{ change: 'moving', words: '1 to another commit' },
		])
		const added = addPick(room(), 'dev/tanks', offered)
		expect('picks' in added && summary(added.picks, room())).toEqual([
			{ change: 'added', words: '1 added' },
		])
		expect(summary(room(), room())).toEqual([])
		expect(sameList(room(), room())).toBe(true)
		expect(sameList([first!], room())).toBe(false)
	})
})

describe('remembered sets', () => {
	test('go to the front as games are played, pinned, once per load order', () => {
		const once = remember([], [sphere, tiny], '2025-10-06T00:00:00Z')
		expect(once[0]?.mods.map((mod) => mod.ref)).toEqual([
			`dev/sphere@${SHA}`,
			'tiny maps v1',
		])
		const other = remember(once, [tiny], '2025-10-08T00:00:00Z')
		expect(other.map((set) => set.at)).toEqual([
			'2025-10-08T00:00:00Z',
			'2025-10-06T00:00:00Z',
		])
		const turned = remember(other, [tiny, sphere], '2025-10-09T00:00:00Z')
		expect(turned.map((set) => set.at)).toEqual([
			'2025-10-09T00:00:00Z',
			'2025-10-08T00:00:00Z',
			'2025-10-06T00:00:00Z',
		])
		const newer = { ...sphere, source: `github:dev/sphere@${OTHER}` }
		const moved = remember(turned, [newer, tiny], 'later')
		expect(moved.map((set) => set.at)).toEqual([
			'later',
			'2025-10-09T00:00:00Z',
			'2025-10-08T00:00:00Z',
		])
		expect(moved[0]?.mods[0]?.ref).toBe(`dev/sphere@${OTHER}`)
		expect(remember(turned, [], 'later')).toEqual(turned)
	})

	test('count only a whole list, every mod here and none twice', () => {
		const building = { ...sphere, here: false, title: 'dev/sphere' }
		expect(remember([], [building], 'then')).toEqual([])
		expect(
			remember([], [sphere, { ...sphere, name: 'again' }], 'then'),
		).toEqual([])
	})

	test('come back as a draft under the names the room knows', () => {
		const [set] = remember([], [sphere, tiny], 'then')
		const draft = adopt(set!.mods, room(), offered)
		expect(draft.map((pick) => pick.ref)).toEqual([
			'sphere-spawner',
			'tiny maps v1',
		])
		const elsewhere = adopt(set!.mods, [], offered)
		expect(elsewhere[0]?.ref).toBe('sphere-spawner')
		expect(adopt(set!.mods, [], [])[0]?.ref).toBe(`dev/sphere@${SHA}`)
	})

	test('survive storage, and nothing else does', () => {
		const held = new Map<string, string>()
		const storage = {
			getItem: (key: string) => held.get(key) ?? null,
			setItem: (key: string, value: string) => void held.set(key, value),
		} as unknown as Storage
		const sets = remember([], [sphere], 'then')
		writeSets(storage, sets)
		expect(readSets(storage)).toEqual(sets)
		storage.setItem(
			'modlobby.modSets',
			'[{"at":1},{"mods":[]},"x",{"at":"a","mods":[{"ref":"r","label":"l"}]}]',
		)
		expect(readSets(storage)).toEqual([
			{ at: 'a', mods: [{ ref: 'r', label: 'l' }] },
		])
		storage.setItem('modlobby.modSets', 'not json')
		expect(readSets(storage)).toEqual([])
		expect(readSets(null)).toEqual([])
	})

	test('stored before these rules, come back tidied', () => {
		const held = new Map<string, string>()
		const storage = {
			getItem: (key: string) => held.get(key) ?? null,
			setItem: (key: string, value: string) => void held.set(key, value),
		} as unknown as Storage
		const [both] = remember([], [sphere, tiny], 'newest')
		const [turned] = remember([], [tiny, sphere], 'older')
		const [alone] = remember([], [tiny], 'oldest')
		const twice = { at: 'mid', mods: [both!.mods[0]!, both!.mods[0]!] }
		const unbuilt = {
			at: 'early',
			mods: [{ ...both!.mods[0]!, label: 'dev/sphere' }],
		}
		writeSets(storage, [both!, twice, turned!, unbuilt, alone!])
		expect(readSets(storage).map((set) => set.at)).toEqual([
			'newest',
			'older',
			'oldest',
		])
	})
})
