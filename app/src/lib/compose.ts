/**
 * Writing a line: recalling the last one, and finishing a word.
 *
 * Both are things every lobby has had for twenty years, and both are worse to
 * live without here than elsewhere: the lines people repeat in a BAR room are
 * SPADS commands like `!bSet tweakdefs1 …`, and the names they address are
 * `[Crd]XxStormKittyxX`.
 *
 * Everything here is a plain function over strings so the behaviour can be
 * tested without a keyboard.
 */

import type { ModOption } from '../ipc/bindings/ModOption'
import { TWEAK_SLOTS } from './setup'

/** How many sent lines are worth keeping. Beyond this nobody is scrolling. */
export const HISTORY_MAX = 60

/**
 * The line to show after pressing up or down.
 *
 * `at` counts back from the end: 0 is the most recent, and `history.length`
 * means "past the oldest", which stays where it is. Coming forward past the
 * newest returns to `draft` — whatever was half-typed when the recall started.
 */
export function recall(
	history: string[],
	at: number,
	step: -1 | 1,
	draft: string,
): { at: number; text: string } {
	const wanted = at + step
	if (wanted < 0) return { at: -1, text: draft }
	if (wanted >= history.length) {
		return at >= history.length
			? { at, text: history[history.length - 1] ?? draft }
			: { at, text: history[history.length - 1 - at] ?? draft }
	}
	return { at: wanted, text: history[history.length - 1 - wanted] ?? draft }
}

/** Adds a sent line, dropping an immediate repeat and anything too old. */
export function remember(history: string[], line: string): string[] {
	const trimmed = line.trim()
	if (!trimmed || history[history.length - 1] === trimmed) return history
	return [...history, trimmed].slice(-HISTORY_MAX)
}

/**
 * What BAR's autohosts answer to: `spads_config_bar/etc/commands*.conf` and
 * its plugins' `*Cmd.conf`, the modded hosts' plugins, and the SPADS settings
 * people type as commands (`allowSettingsShortcut`). Aliases are left out;
 * they are what you type when you already know the name.
 */
// prettier-ignore
export const SPADS_COMMANDS: readonly string[] = [
	'addBot', 'addBox', 'advert', 'aiProfile', 'auth', 'autoBalance', 'balance',
	'balanceAlgorithm', 'ban', 'banIp', 'banIps', 'bKick', 'boss', 'bPreset',
	'bSet', 'callVote', 'cancelQuit', 'cheat', 'chpasswd', 'chrank', 'chskill',
	'cKick', 'clearBox', 'closeBattle', 'clusterStatus', 'dlmap', 'draft',
	'endVote', 'fixColors', 'force', 'forcePreset', 'forceStart', 'gameVersion',
	'gatekeeper', 'getLastVote', 'gKick', 'help', 'helpall', 'hostStats',
	'hPreset', 'hSet', 'joinAs', 'kick', 'kickBan', 'learnMaps', 'list',
	'loadBoxes', 'lock', 'map', 'mapLink', 'maxChevLevel', 'maxRatingLevel',
	'meme', 'minChevLevel', 'minRatingLevel', 'mute', 'mutes', 'mutator',
	'mutators', 'nbTeams', 'nextMap', 'nextPreset', 'notify', 'openBattle',
	'pick', 'plugin', 'preset', 'privateHost', 'promote', 'pSet', 'quit',
	'rebalance', 'rehost', 'reloadArchives', 'reloadConf', 'removeBot', 'rename',
	'resetChevLevels', 'resetRatingLevels', 'resign', 'restart', 'ring',
	'saveBoxes', 'say', 'searchUser', 'send', 'sendLobby', 'set',
	'setAllAiBonus', 'setRatingLevels', 'smurfs', 'specAfk', 'split', 'start',
	'stats', 'status', 'stop', 'teamSize', 'unban', 'unbanIp', 'unbanIps',
	'unboss', 'unlock', 'unlockSpec', 'unmute', 'update', 'version', 'vote',
	'welcome-message', 'whois',
]

/** What a room of your own answers to (`skirmish/src/command.rs`). */
// prettier-ignore
export const SKIRMISH_COMMANDS: readonly string[] = [
	'addBot', 'bSet', 'engine', 'fixColors', 'game', 'help', 'map', 'nbTeams',
	'removeBot', 'rename', 'set', 'start',
]

/** What may finish a word: who is about, and what the room answers to. */
export type Vocabulary = {
	names: readonly string[]
	commands: readonly string[]
	/** Modoption keys. */
	settings: readonly string[]
	/** The room takes `!key value` for `!bSet key value`, as SPADS does. */
	shortcut: boolean
}

/** One way to finish the word under the caret: `lead + key + tail` replaces it. */
export type Completion = {
	/** What Tab puts in front of the key: `!`, `!bSet `, or nothing. */
	lead: string
	/** What the typed word is matched against. */
	key: string
	/** What Tab puts after it: a space, or `: ` to address somebody. */
	tail: string
}

/** A run of what is drawn over the box: letters on the line, or ones Tab would add. */
export type Piece = { text: string; ghost: boolean }

/** The line as the first Tab would leave it, split where the caret is drawn. */
export type Preview = {
	head: Piece[]
	tail: Piece[]
	/**
	 * Tab would put letters before or among the typed ones, so the word is
	 * drawn whole and the box's own text is hidden under it.
	 */
	merged: boolean
}

/**
 * The keys a room's `!bSet` takes: the game's table, then the numbered tweak
 * slots the table builds in a loop and so never lists.
 */
export function settingKeys(options: readonly ModOption[]): string[] {
	const layout = (option: ModOption) =>
		typeof option.type === 'string' && LAYOUT.includes(option.type)
	const keys = options.filter((option) => !layout(option)).map((o) => o.key)
	return [...new Set([...keys, ...TWEAK_SLOTS])]
}

/**
 * Keys whose values only fit behind `!bSet`: teiserver cuts a battle line at
 * 257 characters unless it starts `!bset tweakdefs`, `!bset tweakunits` or
 * `!bset mapmetadata` (`saybattle_max_len`).
 */
const LONG_VALUES = /^(tweakdefs|tweakunits|mapmetadata)/i

/** Rows that draw the options page rather than set anything. */
const LAYOUT: readonly string[] = ['section', 'subheader', 'separator', 'link']

/** The word the caret sits in, and where it starts. */
export function wordAt(
	text: string,
	caret: number,
): { word: string; from: number } {
	const before = text.slice(0, caret)
	const from = before.search(/\S*$/)
	return { word: before.slice(from), from }
}

/** The word under the caret, and what it can be given where it stands. */
function place(text: string, caret: number) {
	const { word, from } = wordAt(text, caret)
	const lineStart = text.lastIndexOf('\n', from - 1) + 1
	const before = text.slice(lineStart, from).split(/\s+/).filter(Boolean)
	const bang = word.startsWith('!')
	return {
		word,
		before,
		bang,
		/** Second after `!bSet`, which is where a modoption's key goes. */
		setting: before.length === 1 && before[0]!.toLowerCase() === '!bset',
		needle: (bang ? word.slice(1) : word).toLowerCase(),
	}
}

/**
 * Everything that could finish the word under the caret, best first.
 *
 * What is offered depends on where the word is. First on its line it may be
 * a command, with or without its `!`, or somebody being addressed; second
 * after `!bSet` it is a modoption; anywhere else it is a name. A command
 * anywhere but the start of a line is just text, so it gets nothing.
 *
 * A modoption is finished as `!key` where the room takes that shortcut, and
 * as `!bSet key` where it does not or the value is one of `LONG_VALUES`.
 *
 * Case-insensitive, because nobody types a name the way it was registered.
 * What was typed exactly comes first, so `!start` is not taken for
 * `!startmetal`; then whatever starts with it, then whatever contains it --
 * typing `sky` means `Skywalker` far more often than `BlueSky` -- and last
 * whatever has its letters in order, so `twd` still finds `tweakdefs`.
 */
export function suggestions(
	text: string,
	caret: number,
	vocabulary: Vocabulary,
): Completion[] {
	const { before, bang, setting, needle } = place(text, caret)
	if (!needle) return []

	const rank = (key: string) => {
		if (key === needle) return 0
		if (key.startsWith(needle)) return 1
		if (key.includes(needle)) return 2
		return letters(needle, key) ? 3 : 4
	}
	// ponytail: tiers only, alphabetical within; score gaps and word starts if fuzzy picks feel random.
	return candidates(before, bang, setting, vocabulary)
		.map((candidate) => ({
			candidate,
			rank: rank(candidate.key.toLowerCase()),
		}))
		.filter(({ rank }) => rank < 4)
		.sort((a, b) => a.rank - b.rank)
		.map(({ candidate }) => candidate)
}

/** What the word could be, given the words before it on its line. */
function candidates(
	before: string[],
	bang: boolean,
	setting: boolean,
	vocabulary: Vocabulary,
): Completion[] {
	const offer = (
		keys: readonly string[],
		lead: (key: string) => string,
		tail = ' ',
	) =>
		[...keys]
			.sort((a, b) => a.localeCompare(b))
			.map((key) => ({ lead: lead(key), key, tail }))
	const bare = () => ''
	const commands = [
		...offer(vocabulary.commands, () => '!'),
		...offer(vocabulary.settings, (key) =>
			vocabulary.shortcut && !LONG_VALUES.test(key) ? '!' : '!bSet ',
		),
	]

	if (before.length === 0 && bang) return commands
	if (before.length === 0)
		return [...offer(vocabulary.names, bare, ': '), ...commands]
	if (bang) return []
	if (setting) return offer(vocabulary.settings, bare)
	return offer(vocabulary.names, bare)
}

/**
 * Where each letter of `needle` falls in `key`, both lower case: in one run
 * where there is one, else each wherever it next turns up. `null` when the
 * letters are not all there in order.
 */
function letters(needle: string, key: string): number[] | null {
	const run = key.indexOf(needle)
	if (run >= 0) return Array.from(needle, (_, i) => run + i)
	const found: number[] = []
	let from = 0
	for (const letter of needle) {
		const at = key.indexOf(letter, from)
		if (at < 0) return null
		found.push(at)
		from = at + 1
	}
	return found
}

/**
 * What to draw over the box for a completion, or nothing when Tab would add
 * no letters.
 *
 * Mostly the line with a ghost of the rest after it: `!ri`, then `ng`. Where
 * Tab would also put letters before or among the typed ones -- `!twea` to
 * `!bSet tweakdefs`, `!bSet twd` to `tweakdefs` -- the word is drawn as it
 * will be, the typed letters among the ghost ones. Only a word that is
 * plainly a command is redrawn like that: a name or a bare word changing
 * shape under the fingers mid-sentence would be noise, so those keep to the
 * plain ghost, and only when the word begins what it finishes.
 */
export function preview(
	text: string,
	caret: number,
	completion: Completion,
): Preview | null {
	const { word, bang, setting, needle } = place(text, caret)
	const { lead, key } = completion
	const at = needle ? letters(needle, key.toLowerCase()) : null
	if (!at) return null

	const plain = (rest: string): Preview | null =>
		rest
			? {
					head: [{ text: text.slice(0, caret), ghost: false }],
					tail: [{ text: rest, ghost: true }],
					merged: false,
				}
			: null

	const shape = drawn(lead, key, at, bang)
	const last = shape.map((piece) => piece.ghost).lastIndexOf(false)
	const tail = shape.slice(last + 1)
	if (!shape.slice(0, last).some((piece) => piece.ghost))
		return plain(tail.map((piece) => piece.text).join(''))
	if (!bang && !setting)
		return key.toLowerCase().startsWith(needle)
			? plain(key.slice(needle.length))
			: null
	return {
		head: [
			{ text: text.slice(0, caret - word.length), ghost: false },
			...shape.slice(0, last + 1),
		],
		tail,
		merged: true,
	}
}

/**
 * The completed word as runs of typed and ghost letters. A typed `!` is the
 * lead's own: every completion offered to a bang word leads with one.
 */
function drawn(
	lead: string,
	key: string,
	at: number[],
	bang: boolean,
): Piece[] {
	const pieces: Piece[] = []
	const add = (text: string, ghost: boolean) => {
		const last = pieces[pieces.length - 1]
		if (last?.ghost === ghost) last.text += text
		else if (text) pieces.push({ text, ghost })
	}
	if (bang) add('!', false)
	add(bang ? lead.slice(1) : lead, true)
	for (let i = 0; i < key.length; i += 1) add(key[i]!, !at.includes(i))
	return pieces
}

/** The line with the word under the caret replaced by a completion. */
export function complete(
	text: string,
	caret: number,
	completion: Completion,
): { text: string; caret: number } {
	const { from } = wordAt(text, caret)
	const insert = completion.lead + completion.key + completion.tail
	const next = text.slice(0, from) + insert + text.slice(caret)
	return { text: next, caret: from + insert.length }
}
