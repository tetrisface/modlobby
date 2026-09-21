import { describe, expect, test } from 'vitest'
import { BOX_OVERRIDE } from './boxes'
import { TWEAK_SLOTS } from './setup'
import {
	SCRATCH,
	SLOT_KEYS,
	defaultCompare,
	defaultTarget,
	draftDoc,
	draftId,
	draftNameFor,
	edit,
	emptyWorkspace,
	firstComment,
	guessKind,
	isDirty,
	kindOf,
	listItems,
	loaded,
	parseSide,
	reset,
	resolveSide,
	searchSlots,
	savedAs,
	sent,
	sideKey,
	sideOptions,
	slotId,
	slotKey,
	slotOf,
	targetOf,
	titleOf,
	unsentCount,
	type Doc,
	type Filter,
	type Side,
} from './tweakspace'

const DEFS = slotId('tweakdefs1')

function slot(over: Partial<Doc> = {}): Doc {
	return { ...emptyWorkspace().docs[DEFS]!, ...over }
}

const arrived = (blob: string, text: string) => ({
	blob,
	text,
	name: firstComment(text),
	summary: `${blob.length}:abcd`,
	notes: [],
})

describe('slots', () => {
	test('every one of the twenty keys parses and prints back', () => {
		for (const key of TWEAK_SLOTS) {
			const parsed = slotOf(key)
			expect(parsed).not.toBeNull()
			expect(slotKey(parsed!)).toBe(key)
			expect(parsed!.kind).toBe(kindOf(key))
		}
		expect(slotOf('tweakdefs')).toEqual({ kind: 'defs', index: 0 })
		expect(slotOf('tweakunits9')).toEqual({ kind: 'units', index: 9 })
		// Past the ten written from here: BAR reads any index (#6597).
		expect(slotOf('tweakunits29')).toEqual({ kind: 'units', index: 29 })
		expect(slotOf('tweakdefs01')).toBeNull()
		expect(slotOf('tweakdefs256')).toBeNull()
		expect(slotOf('startmetal')).toBeNull()
	})

	test('the start-box override is the twenty-first, of its own kind', () => {
		expect(SLOT_KEYS).toEqual([...TWEAK_SLOTS, BOX_OVERRIDE])
		expect(slotOf(BOX_OVERRIDE)).toEqual({ kind: 'boxes' })
		expect(slotKey({ kind: 'boxes' })).toBe(BOX_OVERRIDE)
		expect(kindOf(BOX_OVERRIDE)).toBe('boxes')
		expect(defaultTarget('boxes')).toBe(BOX_OVERRIDE)
		expect(slotOf('mapmetadata_startboxes_set')).toBeNull()
	})

	test('ids carry their title', () => {
		expect(titleOf(slotId('tweakunits2'))).toBe('tweakunits2')
		expect(titleOf(draftId('my:draft'))).toBe('my:draft')
	})
})

describe('guessKind', () => {
	test('a table is units, code is defs, and a header does not decide', () => {
		expect(guessKind('{ armcom = { metalcost = 1 } }')).toBe('units')
		expect(guessKind('-- Named\n-- by me\n{ armcom = {} }')).toBe('units')
		expect(guessKind('local x = 1')).toBe('defs')
		expect(guessKind('-- Named\nfor name, ud in pairs(UnitDefs) do end')).toBe(
			'defs',
		)
		expect(guessKind('')).toBe('defs')
	})
})

describe('loading a slot', () => {
	test('a clean document becomes the room value', () => {
		const doc = loaded(slot(), arrived('YQ==', '-- Nutty\nlocal a = 1\n'))
		expect(doc.loaded).toBe(true)
		expect(doc.buffer).toBe('-- Nutty\nlocal a = 1\n')
		expect(doc.original).toBe(doc.buffer)
		expect(doc.name).toBe('Nutty')
		expect(isDirty(doc)).toBe(false)
		expect(doc.stale).toBe(false)
	})

	test('a dirty document keeps its edit and is stale once the room moves', () => {
		const before = loaded(slot(), arrived('YQ==', 'local a = 1\n'))
		const typed = edit(before, 'local a = 2\n')
		expect(isDirty(typed)).toBe(true)

		// The same blob decoded again: nothing moved.
		const same = loaded(typed, arrived('YQ==', 'local a = 1\n'))
		expect(same.buffer).toBe('local a = 2\n')
		expect(same.stale).toBe(false)

		const moved = loaded(typed, arrived('Yg==', 'local b = 1\n'))
		expect(moved.buffer).toBe('local a = 2\n')
		expect(moved.original).toBe('local b = 1\n')
		expect(moved.stale).toBe(true)
	})

	test('reset returns to the room and clears stale; sent adopts the buffer', () => {
		const doc = edit(loaded(slot(), arrived('YQ==', 'local a = 1\n')), 'x')
		const stale = loaded(doc, arrived('Yg==', 'local b = 1\n'))
		expect(reset(stale)).toMatchObject({
			buffer: 'local b = 1\n',
			stale: false,
		})
		const gone = sent(edit(stale, '-- Mine\nlocal c = 1\n'))
		expect(gone).toMatchObject({
			original: '-- Mine\nlocal c = 1\n',
			stale: false,
			name: 'Mine',
		})
	})

	test('typing what is already there is not an edit', () => {
		const doc = loaded(slot(), arrived('YQ==', 'a'))
		expect(edit(doc, 'a')).toBe(doc)
	})
})

describe('drafts', () => {
	test('a draft is loaded, named by its header, and typed by its shape', () => {
		const draft = draftDoc('walls', '-- T3 walls\n{ armwall = {} }')
		expect(draft).toMatchObject({
			id: 'draft:walls',
			origin: 'draft',
			kind: 'units',
			name: 'T3 walls',
			loaded: true,
		})
		expect(isDirty(draft)).toBe(false)
	})

	test('a draft is named by the header being typed before the one the room holds', () => {
		expect(draftNameFor(slot({ name: 'Room', buffer: '-- Typed\nx' }))).toBe(
			'Typed',
		)
		expect(draftNameFor(slot({ name: 'Room', buffer: 'x' }))).toBe('Room')
		expect(draftNameFor(slot({ buffer: 'x' }))).toBe('tweakdefs1')
		expect(draftNameFor(draftDoc('walls', '-- T3\n{}'))).toBe('walls')
	})

	test('saving a slot as a draft keeps the slot kind even for an empty buffer', () => {
		const doc = slot({ kind: 'units', buffer: '' })
		expect(savedAs(doc, 'blank')).toMatchObject({
			id: 'draft:blank',
			kind: 'units',
		})
	})
})

describe('the list', () => {
	const ws = (() => {
		const base = emptyWorkspace()
		const docs = { ...base.docs }
		docs[slotId('tweakunits2')] = loaded(
			docs[slotId('tweakunits2')]!,
			arrived('e30=', '-- Golem\n{}'),
		)
		docs[DEFS] = edit(
			loaded(docs[DEFS]!, arrived('YQ==', 'local a = 1')),
			'local a = 2',
		)
		// SPADS cleared it: the `0` it writes is not a value.
		docs[slotId(BOX_OVERRIDE)] = loaded(
			docs[slotId(BOX_OVERRIDE)]!,
			arrived('0', ''),
		)
		docs[draftId('b-draft')] = draftDoc('b-draft', 'local b')
		docs[draftId('a-draft')] = draftDoc('a-draft', '{ x = 1 }')
		return { ...base, docs }
	})()

	const titles = (filter: Filter) =>
		listItems(ws, filter).map((item) => item.title)

	test('the scratch comes first, then the drafts, measured in Lua', () => {
		const items = listItems(ws)
		expect(items.map((item) => item.title)).toEqual([
			'untitled',
			'a-draft',
			'b-draft',
		])
		expect(items[0]).toMatchObject({ id: SCRATCH, empty: true, size: 0 })
		expect(items[1]).toMatchObject({ kind: 'units', size: 9, dirty: false })
	})

	test('search matches the file name or the header, and never hides the scratch', () => {
		const named = {
			...ws,
			docs: { ...ws.docs, [draftId('c')]: draftDoc('c', '-- Golem\n{}') },
		}
		expect(
			listItems(named, { query: 'GOLEM', sort: 'name' }).map(
				(item) => item.title,
			),
		).toEqual(['untitled', 'c'])
		expect(titles({ query: 'b-dr', sort: 'name' })).toEqual([
			'untitled',
			'b-draft',
		])
	})

	test('sorting by kind puts units first, as BAR runs them; by name uses the header', () => {
		expect(titles({ query: '', sort: 'kind' })).toEqual([
			'untitled',
			'a-draft',
			'b-draft',
		])
		expect(titles({ query: '', sort: 'name' })).toEqual([
			'untitled',
			'a-draft',
			'b-draft',
		])
	})

	test('the scratch is named by its header as it is typed', () => {
		const typed = {
			...ws,
			docs: { ...ws.docs, [SCRATCH]: edit(ws.docs[SCRATCH]!, '-- Mine\nx') },
		}
		expect(listItems(typed)[0]).toMatchObject({ name: 'Mine', dirty: true })
	})

	test('counts only the slots holding an edit the room has not seen', () => {
		expect(unsentCount(ws)).toBe(1)
		const drafted = {
			...ws,
			docs: {
				...ws.docs,
				[draftId('a-draft')]: edit(ws.docs[draftId('a-draft')]!, 'y'),
				[SCRATCH]: edit(ws.docs[SCRATCH]!, 'x'),
			},
		}
		expect(unsentCount(drafted)).toBe(1)
	})
})

describe('target', () => {
	test('a slot is sent to itself; a draft to the chosen slot', () => {
		const ws = emptyWorkspace(slotId('tweakunits3'))
		expect(targetOf(ws)).toEqual({ kind: 'units', index: 3 })
		const docs = { ...ws.docs, [draftId('d')]: draftDoc('d', 'x') }
		expect(
			targetOf({ ...ws, docs, active: draftId('d'), target: 'tweakdefs4' }),
		).toEqual({ kind: 'defs', index: 4 })
	})
})

describe('searching every tweak', () => {
	const ws = (() => {
		const base = emptyWorkspace()
		const docs = { ...base.docs }
		const hold = (key: string, text: string) => {
			docs[slotId(key)] = loaded(docs[slotId(key)]!, arrived('YQ==', text))
		}
		hold(
			'tweakdefs1',
			'-- Walls\nUnitDefs.armwall.health = 1\n-- ARMWALL again: armwall\n',
		)
		hold('tweakunits', '{\n\tarmwall = { health = 12000 },\n}\n')
		hold('tweakdefs2', 'local nothing = 1\n')
		hold(BOX_OVERRIDE, '{"armwall": 1}')
		docs[draftId('walls')] = draftDoc('walls', 'armwall')
		return { ...base, docs }
	})()

	test('finds every match in every room tweak, units first, case aside', () => {
		const found = searchSlots(ws, 'ArmWall')
		expect(found.map((entry) => entry.title)).toEqual([
			'tweakunits',
			'tweakdefs1',
		])
		expect(found[0]!.hits).toEqual([
			{
				line: 2,
				column: 2,
				length: 7,
				text: '\tarmwall = { health = 12000 },',
			},
		])
		expect(found[1]!.name).toBe('Walls')
		expect(found[1]!.hits.map((hit) => [hit.line, hit.column])).toEqual([
			[2, 10],
			[3, 4],
			[3, 19],
		])
	})

	test('an unsent edit is what is searched, not what the room holds', () => {
		const edited = {
			...ws,
			docs: {
				...ws.docs,
				[slotId('tweakdefs2')]: edit(ws.docs[slotId('tweakdefs2')]!, 'armwall'),
			},
		}
		expect(
			searchSlots(edited, 'armwall').map((entry) => entry.title),
		).toContain('tweakdefs2')
	})

	test('leaves out the override, drafts, and an empty query', () => {
		expect(searchSlots(ws, '')).toEqual([])
		const titles = searchSlots(ws, 'armwall').map((entry) => entry.title)
		expect(titles).not.toContain(BOX_OVERRIDE)
		expect(titles).not.toContain('walls')
	})

	test('lists a hundred matches in a tweak, and counts the rest', () => {
		const many = {
			...ws,
			docs: {
				...ws.docs,
				[slotId('tweakdefs3')]: edit(
					ws.docs[slotId('tweakdefs3')]!,
					'x\n'.repeat(130),
				),
			},
		}
		const found = searchSlots(many, 'x')
		expect(found[0]).toMatchObject({ title: 'tweakdefs3', more: 30 })
		expect(found[0]!.hits).toHaveLength(100)
	})
})

describe('comparing', () => {
	const ws = (() => {
		const base = emptyWorkspace(slotId('tweakdefs1'))
		const docs = { ...base.docs }
		docs[DEFS] = edit(
			loaded(docs[DEFS]!, arrived('YQ==', 'local a = 1')),
			'local a = 2',
		)
		docs[slotId('tweakunits2')] = loaded(
			docs[slotId('tweakunits2')]!,
			arrived('e30=', '{}'),
		)
		// A cleared override is not a side worth offering.
		docs[slotId(BOX_OVERRIDE)] = loaded(
			docs[slotId(BOX_OVERRIDE)]!,
			arrived('0', ''),
		)
		docs[draftId('walls')] = draftDoc('walls', '{ armwall = {} }')
		return { ...base, docs }
	})()
	const history = [
		{ seq: 6, key: 'allowpausegameplay', from: '0', to: '1', by: null },
		{ seq: 7, key: 'tweakdefs1', from: 'YQ==', to: 'Yg==', by: 'Lathek' },
		{
			seq: 8,
			key: 'mapmetadata_startbox_override',
			from: '',
			to: 'eJyr',
			by: 'Host',
		},
	]

	test('every side survives the trip through a select value', () => {
		const sides: Side[] = [
			{ doc: DEFS, text: 'buffer' },
			{ doc: draftId('a:b'), text: 'original' },
			{ history: 7, which: 'from' },
			{ vote: true },
		]
		for (const side of sides) expect(parseSide(sideKey(side))).toEqual(side)
		expect(parseSide('doc:sideways:slot:x')).toBeNull()
		expect(parseSide('history:from:seven')).toBeNull()
		expect(parseSide('nonsense')).toBeNull()
	})

	test('the menu lists held slots, edited buffers, drafts, slot changes and the vote', () => {
		const options = sideOptions(ws, history, 'Yg==')
		expect(options.map((option) => option.label)).toEqual([
			'tweakunits2 · room',
			'tweakdefs1 · room',
			'tweakdefs1 · edited',
			'walls · file',
			'#7 tweakdefs1 before by Lathek',
			'#7 tweakdefs1 after by Lathek',
			'#8 mapmetadata_startbox_override before by Host',
			'#8 mapmetadata_startbox_override after by Host',
			'what the vote proposes',
		])
		expect(options[3]!.group).toBe('Drafts')
		expect(sideOptions(ws, [], null).map((o) => o.group)).not.toContain('Vote')
	})

	test('a side resolves to Lua for a document and to a blob for the rest', () => {
		expect(
			resolveSide(ws, { doc: DEFS, text: 'buffer' }, history, null),
		).toEqual({
			label: 'tweakdefs1 · edited',
			kind: 'defs',
			lua: 'local a = 2',
		})
		expect(resolveSide(ws, { history: 7, which: 'to' }, history, null)).toEqual(
			{ label: '#7 tweakdefs1 after', kind: 'defs', blob: 'Yg==' },
		)
		// The override's history decodes as boxes, which is what makes it JSON.
		expect(
			resolveSide(ws, { history: 8, which: 'to' }, history, null),
		).toMatchObject({ kind: 'boxes', blob: 'eJyr' })
		expect(resolveSide(ws, { vote: true }, history, 'Yg==')).toMatchObject({
			kind: 'defs',
			blob: 'Yg==',
		})
		expect(resolveSide(ws, { vote: true }, history, null)).toBeNull()
		expect(
			resolveSide(ws, { history: 99, which: 'to' }, history, null),
		).toBeNull()
	})

	test('opening compare shows the edit against what was loaded, or a clean document against itself', () => {
		expect(defaultCompare(ws)).toEqual({
			left: { doc: DEFS, text: 'original' },
			right: { doc: DEFS, text: 'buffer' },
		})
		const clean = { ...ws, active: slotId('tweakunits2') }
		expect(defaultCompare(clean).right).toEqual({
			doc: slotId('tweakunits2'),
			text: 'original',
		})
	})
})
