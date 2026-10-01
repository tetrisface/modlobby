import { describe, expect, test } from 'vitest'
import {
	HISTORY_MAX,
	complete,
	preview,
	recall,
	remember,
	settingKeys,
	suggestions,
	wordAt,
	type Piece,
	type Vocabulary,
} from './compose'
import { fixtureOptions } from './setup.fixture'

describe('remembering what was sent', () => {
	test('lines are kept in the order they were sent', () => {
		let history: string[] = []
		history = remember(history, '!balance')
		history = remember(history, 'gg')
		expect(history).toEqual(['!balance', 'gg'])
	})

	test('blank lines and immediate repeats are not worth keeping', () => {
		expect(remember(['gg'], '   ')).toEqual(['gg'])
		expect(remember(['gg'], 'gg')).toEqual(['gg'])
		// ...but the same line again later is a real recall target.
		expect(remember(['gg', 'hi'], 'gg')).toEqual(['gg', 'hi', 'gg'])
	})

	test('the oldest fall off once there are too many', () => {
		let history: string[] = []
		for (let n = 0; n < HISTORY_MAX + 10; n += 1)
			history = remember(history, `line ${n}`)
		expect(history.length).toBe(HISTORY_MAX)
		expect(history[0]).toBe('line 10')
	})
})

describe('walking back through it', () => {
	const history = ['first', 'second', 'third']

	test('up walks back from the newest', () => {
		let step = recall(history, -1, 1, 'draft')
		expect(step).toEqual({ at: 0, text: 'third' })
		step = recall(history, step.at, 1, 'draft')
		expect(step).toEqual({ at: 1, text: 'second' })
		step = recall(history, step.at, 1, 'draft')
		expect(step).toEqual({ at: 2, text: 'first' })
	})

	test('the oldest is where it stops', () => {
		expect(recall(history, 2, 1, 'draft')).toEqual({ at: 2, text: 'first' })
	})

	test('down comes back, and past the newest is what you were typing', () => {
		expect(recall(history, 1, -1, 'draft')).toEqual({ at: 0, text: 'third' })
		expect(recall(history, 0, -1, 'draft')).toEqual({ at: -1, text: 'draft' })
	})

	test('an empty history leaves the draft alone', () => {
		expect(recall([], -1, 1, 'draft')).toEqual({ at: -1, text: 'draft' })
	})
})

const words: Vocabulary = {
	names: [
		'Skywalker',
		'BlueSky',
		'sky_bot',
		'[Crd]XxStormKittyxX',
		'tetrisface',
	],
	commands: ['ring', 'bSet', 'balance', 'start'],
	settings: ['tweakdefs', 'tweakdefs1', 'startmetal', 'mapmetadata'],
	shortcut: true,
}

/** What each offer would put in place of the word, best first. */
const offered = (text: string, vocabulary: Vocabulary = words) =>
	suggestions(text, text.length, vocabulary).map((c) => c.lead + c.key + c.tail)

describe('finishing a name', () => {
	test('a prefix match is offered before a mere containment', () => {
		expect(offered('hi sky')).toEqual([
			'sky_bot ',
			'Skywalker ',
			'BlueSky ',
			// s…k…y, last: its letters are there, in order, but apart.
			'[Crd]XxStormKittyxX ',
		])
	})

	test('case does not matter, since nobody types a name as registered', () => {
		expect(offered('hi SKYW')).toEqual(['Skywalker '])
		expect(offered('hi xxstorm')).toEqual(['[Crd]XxStormKittyxX '])
	})

	test('nothing to go on offers nothing', () => {
		expect(offered('hi ')).toEqual([])
		expect(offered('hi zzz')).toEqual([])
		expect(offered('!')).toEqual([])
	})

	test('a whole name is offered as registered', () => {
		expect(offered('hi skywalker')).toEqual(['Skywalker '])
	})

	test("a command's argument is a name", () => {
		expect(offered('!ring tet')).toEqual(['tetrisface '])
	})

	test('at the start of a line, a name is being addressed', () => {
		expect(offered('tet')[0]).toBe('tetrisface: ')
		expect(offered('!preset team\ntet')[0]).toBe('tetrisface: ')
	})
})

describe('finishing a command', () => {
	test('a bang word at the start of a line is a command or a modoption', () => {
		expect(offered('!rin')).toEqual(['!ring '])
		expect(offered('!sta')).toEqual(['!start ', '!startmetal '])
	})

	test('a long value goes through !bSet, which teiserver does not cut', () => {
		expect(offered('!twea')).toEqual(['!bSet tweakdefs ', '!bSet tweakdefs1 '])
		expect(offered('!mapm')).toEqual(['!bSet mapmetadata '])
		// Whole, but still worth a Tab: the shortcut is what gets cut off.
		expect(offered('!tweakdefs')[0]).toBe('!bSet tweakdefs ')
	})

	test('a room without the shortcut gets !bSet for every modoption', () => {
		expect(offered('!startm', { ...words, shortcut: false })).toEqual([
			'!bSet startmetal ',
		])
	})

	test('without the bang, Tab puts it in front', () => {
		expect(offered('rin')).toEqual(['!ring '])
		expect(offered('ring')).toEqual(['!ring '])
	})

	test('names come first where a word could be either', () => {
		expect(offered('s').slice(0, 4)).toEqual([
			'sky_bot: ',
			'Skywalker: ',
			'!start ',
			'!startmetal ',
		])
	})

	test('what was typed exactly wins over anything longer', () => {
		expect(offered('!start')).toEqual(['!start ', '!startmetal '])
		expect(offered('!ring')).toEqual(['!ring '])
	})

	test('mid-line a bang word is just text', () => {
		expect(offered('hello !ri')).toEqual([])
	})

	test("after !bSet comes the modoption's key", () => {
		expect(offered('!bSet twea')).toEqual(['tweakdefs ', 'tweakdefs1 '])
		expect(offered('!bset tweakdefs')).toEqual(['tweakdefs ', 'tweakdefs1 '])
	})

	test('letters in order find a key after everything closer', () => {
		expect(offered('!twd')).toEqual(['!bSet tweakdefs ', '!bSet tweakdefs1 '])
		expect(offered('!stm')[0]).toBe('!startmetal ')
		expect(offered('!dwt')).toEqual([])
	})
})

describe('what is drawn over the box', () => {
	/** Ghost letters in brackets, the caret as `|`. */
	const drawn = (text: string) => {
		const first = suggestions(text, text.length, words)[0]
		const picture = first ? preview(text, text.length, first) : null
		if (!picture) return null
		const show = (pieces: Piece[]) =>
			pieces.map((p) => (p.ghost ? `[${p.text}]` : p.text)).join('')
		return {
			drawn: `${show(picture.head)}|${show(picture.tail)}`,
			merged: picture.merged,
		}
	}

	test('the rest of a word it begins, after what was typed', () => {
		expect(drawn('!ring tet')).toEqual({
			drawn: '!ring tet|[risface]',
			merged: false,
		})
		expect(drawn('TET')).toEqual({ drawn: 'TET|[risface]', merged: false })
	})

	test('what Tab adds in front is drawn in place', () => {
		expect(drawn('!twea')).toEqual({
			drawn: '![bSet ]twea|[kdefs]',
			merged: true,
		})
	})

	test('letters in order are drawn among the ghost ones', () => {
		expect(drawn('!bSet twd')).toEqual({
			drawn: '!bSet tw[eak]d|[efs]',
			merged: true,
		})
		expect(drawn('!twd')).toEqual({
			drawn: '![bSet ]tw[eak]d|[efs]',
			merged: true,
		})
	})

	test('nothing once the word is whole', () => {
		expect(drawn('!start')).toBeNull()
	})

	test('a name or a bare word keeps to the plain ghost', () => {
		expect(drawn('hi xxstorm')).toBeNull()
		expect(drawn('rin')).toEqual({ drawn: 'rin|[g]', merged: false })
		expect(drawn('twd')).toBeNull()
	})
})

describe('what !bSet takes', () => {
	test("the game's keys and every tweak slot, without the page's layout rows", () => {
		const keys = settingKeys(fixtureOptions())
		expect(keys).toContain('tweakdefs3')
		expect(keys).toContain('tweakunits')
		const layout = fixtureOptions().filter((o) => o.type === 'section')
		expect(layout.length).toBeGreaterThan(0)
		for (const row of layout) expect(keys).not.toContain(row.key)
	})
})

describe('putting the completion into the line', () => {
	test('the word under the caret is what gets replaced', () => {
		expect(wordAt('hey sky', 7)).toEqual({ word: 'sky', from: 4 })
		expect(wordAt('sky', 3)).toEqual({ word: 'sky', from: 0 })
		expect(wordAt('!preset team\n!bS', 16)).toEqual({ word: '!bS', from: 13 })
	})

	test('the word is replaced by the whole insert', () => {
		expect(
			complete('!twea', 5, { lead: '!bSet ', key: 'tweakdefs', tail: ' ' }),
		).toEqual({ text: '!bSet tweakdefs ', caret: 16 })
	})

	test('whatever follows the caret is kept', () => {
		expect(
			complete('sky and rest', 3, { lead: '', key: 'Skywalker', tail: ': ' }),
		).toEqual({ text: 'Skywalker:  and rest', caret: 11 })
	})
})
