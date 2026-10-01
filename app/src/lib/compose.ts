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

/** One way to finish the word under the caret. */
export type Completion = {
	/** What the typed word was matched against; the ghost is the rest of it. */
	key: string
	/** What replaces the typed word, separator included. */
	insert: string
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
 * `!startmetal`; then whatever starts with it, then whatever merely contains
 * it: typing `sky` means `Skywalker` far more often than `BlueSky`.
 */
export function suggestions(
	text: string,
	caret: number,
	vocabulary: Vocabulary,
): Completion[] {
	const { word, from } = wordAt(text, caret)
	const lineStart = text.lastIndexOf('\n', from - 1) + 1
	const before = text.slice(lineStart, from).split(/\s+/).filter(Boolean)
	const bang = word.startsWith('!')
	const needle = (bang ? word.slice(1) : word).toLowerCase()
	if (!needle) return []

	const rank = ({ key }: Completion) => {
		const lower = key.toLowerCase()
		if (lower === needle) return 0
		if (lower.startsWith(needle)) return 1
		return lower.includes(needle) ? 2 : 3
	}
	return candidates(before, bang, vocabulary)
		.filter((candidate) => rank(candidate) < 3)
		.sort((a, b) => rank(a) - rank(b))
}

/** What the word could be, given the words before it on its line. */
function candidates(
	before: string[],
	bang: boolean,
	vocabulary: Vocabulary,
): Completion[] {
	const offer = (keys: readonly string[], insert: (key: string) => string) =>
		[...keys]
			.sort((a, b) => a.localeCompare(b))
			.map((key) => ({ key, insert: insert(key) }))
	const settable = (key: string) =>
		vocabulary.shortcut && !LONG_VALUES.test(key) ? `!${key} ` : `!bSet ${key} `
	const commands = [
		...offer(vocabulary.commands, (key) => `!${key} `),
		...offer(vocabulary.settings, settable),
	]

	if (before.length === 0 && bang) return commands
	if (before.length === 0)
		return [...offer(vocabulary.names, (name) => `${name}: `), ...commands]
	if (bang) return []
	if (before.length === 1 && before[0]!.toLowerCase() === '!bset')
		return offer(vocabulary.settings, (key) => `${key} `)
	return offer(vocabulary.names, (name) => `${name} `)
}

/**
 * The untyped rest of a completion, drawn faintly after the caret. Empty for
 * one the typed word does not begin: that needs its letters drawn in among
 * the typed ones, which a ghost after the caret cannot do.
 */
export function ghost(
	text: string,
	caret: number,
	completion: Completion,
): string {
	const { word } = wordAt(text, caret)
	const typed = word.startsWith('!') ? word.slice(1) : word
	return completion.key.toLowerCase().startsWith(typed.toLowerCase())
		? completion.key.slice(typed.length)
		: ''
}

/** The line with the word under the caret replaced by a completion. */
export function complete(
	text: string,
	caret: number,
	completion: Completion,
): { text: string; caret: number } {
	const { from } = wordAt(text, caret)
	const next = text.slice(0, from) + completion.insert + text.slice(caret)
	return { text: next, caret: from + completion.insert.length }
}
