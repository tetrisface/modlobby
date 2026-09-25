import type { MutatorView } from '../ipc/bindings/MutatorView'
import { type Offer, sameMutator, sourceWords } from './mutators'
import { move } from './reorder'

/**
 * A mod in a list of what a room should load -- the room's own list, or a
 * draft of the next one: how the host is asked for it, and what to show.
 *
 * The host takes the whole list in one command (`!mutator set a, b, c`), so
 * a draft is edited freely here and costs one vote when it goes.
 */
export type Pick = {
	/**
	 * What `!mutator set` names it by: a name the host offers, or a GitHub
	 * repository -- `owner/repo` for the newest commit of the branch it
	 * follows, `owner/repo@<branch or commit>` for a particular one.
	 */
	ref: string
	/** What a person reads: the archive's own name once built, else the repository's. */
	label: string
	/** `owner/repo`, for one from GitHub. */
	repo: string | null
	/** `github:owner/repo@<commit>` for one the room has pinned. */
	source: string | null
	date: string | null
}

/** What a room loads at most; the host refuses more (`MAX_MUTATORS`). */
export const MOST = 10

/** How many sets are kept, newest first. */
const KEPT = 12

const REPO = '[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+'
const REF = '[A-Za-z0-9_./-]+'
const REPO_REF = new RegExp(`^(${REPO})(?:@(${REF}))?$`)
const GITHUB_URL = new RegExp(
	`^(?:https?://)?(?:www\\.)?github\\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\\.git)?(?:/tree/(${REF}))?/*$`,
)
const COMMIT = /^[0-9a-f]{40}$/

/**
 * `owner/repo` and the branch, tag or commit asked for, out of what somebody
 * pasted: the repository's page on GitHub, with or without `/tree/<branch>`,
 * or `owner/repo[@ref]` as the host takes it.
 */
export function parseGithub(
	text: string,
): { repo: string; ref: string | null } | null {
	const found = GITHUB_URL.exec(text.trim()) ?? REPO_REF.exec(text.trim())
	const [, repo, ref] = found ?? []
	if (!repo || ref?.includes('..')) return null
	return { repo, ref: ref ?? null }
}

/** `owner/repo@<commit>` out of `github:owner/repo@<commit>`. */
function pinRef(source: string | null): string | null {
	return source?.startsWith('github:') ? source.slice('github:'.length) : null
}

function repoOfSource(source: string | null): string | null {
	return pinRef(source)?.split('@')[0] ?? null
}

const key = (name: string) => name.split(/\s+/).join(' ').toLowerCase()

/** The room's loaded mods as picks, each by the name its host knows it by. */
export function picksOf(
	loaded: readonly MutatorView[],
	offered: readonly Offer[],
): Pick[] {
	return loaded.map((mutator) => ({
		ref:
			offered.find((offer) => sameMutator(mutator, offer))?.name ??
			pinRef(mutator.source) ??
			mutator.name,
		label: mutator.title,
		repo: repoOfSource(mutator.source),
		source: mutator.source,
		date: mutator.date,
	}))
}

/** Whether a pick is this offer: by repository for one from GitHub, else by name. */
function isOffer(pick: Pick, offer: Offer): boolean {
	const repo = repoOfSource(offer.source)
	return repo ? pick.repo === repo : key(pick.ref) === key(offer.name)
}

/** The offer, if any, that a pick or a repository stands for. */
export function offerOf(
	picks: readonly Pick[],
	offer: Offer,
): Pick | undefined {
	return picks.find((pick) => isOffer(pick, offer))
}

/**
 * The list with what somebody typed or pasted at the end: a name the host
 * offers, else a GitHub repository. Or why it will not go in.
 */
export function addPick(
	picks: readonly Pick[],
	text: string,
	offered: readonly Offer[],
): { picks: Pick[] } | { problem: string } {
	const offer = offered.find((entry) => key(entry.name) === key(text))
	const github = offer ? null : parseGithub(text)
	if (!offer && !github)
		return {
			problem:
				'Not a name the host offers, nor a GitHub repository (owner/repo or its page)',
		}
	const pick: Pick = offer
		? {
				ref: offer.name,
				label: offer.name,
				repo: repoOfSource(offer.source),
				source: offer.source,
				date: offer.date,
			}
		: {
				ref: github!.ref ? `${github!.repo}@${github!.ref}` : github!.repo,
				label: github!.repo.split('/')[1] ?? github!.repo,
				repo: github!.repo,
				source: null,
				date: null,
			}
	const twice = picks.find(
		(held) =>
			key(held.ref) === key(pick.ref) || (pick.repo && held.repo === pick.repo),
	)
	if (twice) return { problem: `${twice.label} is already in the list` }
	if (picks.length >= MOST)
		return { problem: `A room loads at most ${MOST} mods` }
	return { picks: [...picks, pick] }
}

export function removePick(picks: readonly Pick[], index: number): Pick[] {
	return picks.filter((_, at) => at !== index)
}

export function movePick(
	picks: readonly Pick[],
	from: number,
	to: number,
): Pick[] {
	return move(picks, from, to)
}

/**
 * The same mod at the newest commit of the branch it follows, which the host
 * resolves when the list goes. Only one from GitHub can move.
 */
export function updatePick(pick: Pick): Pick {
	return pick.repo ? { ...pick, ref: pick.repo } : pick
}

/**
 * A pick with its source replaced by what somebody typed: another branch or
 * commit of the same repository keeps what the room knows of it, another
 * repository is another mod. `null` for text that names no repository.
 */
export function editPick(pick: Pick, text: string): Pick | null {
	const github = parseGithub(text)
	if (!github) return null
	const same = github.repo === pick.repo
	return {
		ref: github.ref ? `${github.repo}@${github.ref}` : github.repo,
		label: same ? pick.label : (github.repo.split('/')[1] ?? github.repo),
		repo: github.repo,
		source: same ? pick.source : null,
		date: same ? pick.date : null,
	}
}

/**
 * Where a GitHub pick stands, for a person: the commit the room has, and
 * where the ref would move it -- `dev/sphere @ 9108a17 → newest of main`.
 */
export function sourceLine(pick: Pick): string | null {
	if (!pick.repo) return null
	const held = pick.source ? sourceWords(pick.source) : pick.repo
	if (pick.ref === pinRef(pick.source) || !parseGithub(pick.ref)) return held
	const asked = pick.ref.split('@')[1] ?? null
	if (asked === null) return `${held} → newest`
	if (COMMIT.test(asked)) return `${held} → ${asked.slice(0, 7)}`
	return `${held} → newest of ${asked}`
}

/** How a pick in a draft differs from the room: not at all, new, or moved to another commit. */
export type Change = 'same' | 'added' | 'moving'

export function changeOf(pick: Pick, room: readonly Pick[]): Change {
	if (room.some((held) => key(held.ref) === key(pick.ref))) return 'same'
	if (pick.repo && room.some((held) => held.repo === pick.repo)) return 'moving'
	return 'added'
}

/** Whether two lists ask the host for the same thing. */
export function sameList(a: readonly Pick[], b: readonly Pick[]): boolean {
	return a.length === b.length && a.every((pick, at) => pick.ref === b[at]?.ref)
}

/** The one command that makes the room load exactly this list. */
export function command(picks: readonly Pick[]): string {
	if (picks.length === 0) return '!mutator clear'
	return `!mutator set ${picks.map((pick) => pick.ref).join(', ')}`
}

/** One thing a draft changes, in a few words, and which kind it is. */
export type SummaryPart = {
	change: 'added' | 'removed' | 'moving' | 'reordered'
	words: string
}

/**
 * What a draft changes: `1 added`, `2 removed`, `1 to another commit`, or
 * only `reordered`. Nothing, when it asks for what the room has.
 */
export function summary(
	draft: readonly Pick[],
	room: readonly Pick[],
): SummaryPart[] {
	const changes = draft.map((pick) => changeOf(pick, room))
	const count = (change: Change) => changes.filter((c) => c === change).length
	const removed = room.filter(
		(held) =>
			!draft.some(
				(pick) =>
					key(pick.ref) === key(held.ref) ||
					(held.repo && pick.repo === held.repo),
			),
	).length
	const parts = (
		[
			['added', count('added'), 'added'],
			['removed', removed, 'removed'],
			['moving', count('moving'), 'to another commit'],
		] as const
	)
		.filter(([, n]) => n > 0)
		.map(([change, n, words]) => ({ change, words: `${n} ${words}` }))
	if (parts.length === 0 && !sameList(draft, room))
		return [{ change: 'reordered', words: 'reordered' }]
	return parts
}

/** A list a room loaded while you were in it, with when it did. */
export type ModSet = { at: string; mods: Pick[] }

const signature = (mods: readonly Pick[]) =>
	mods.map((mod) => mod.ref).join('\n')

const labels = (mods: readonly Pick[]) =>
	mods.map((mod) => mod.label).join('\n')

/**
 * The sets with what a room loads now at the front, each mod pinned to the
 * commit it had, so the set means the same later. Only once every mod is
 * here: before then a mod from GitHub has no name but its repository's. A
 * set already at the front keeps its place and takes the names; one further
 * back moves up.
 */
export function remember(
	sets: readonly ModSet[],
	loaded: readonly MutatorView[],
	at: string,
): ModSet[] {
	if (loaded.length === 0 || loaded.some((mutator) => !mutator.here))
		return [...sets]
	const mods = loaded.map((mutator) => ({
		ref: pinRef(mutator.source) ?? mutator.name,
		label: mutator.title,
		repo: repoOfSource(mutator.source),
		source: mutator.source,
		date: mutator.date,
	}))
	const front = sets[0]
	if (front && signature(front.mods) === signature(mods))
		return labels(front.mods) === labels(mods)
			? [...sets]
			: [{ at: front.at, mods }, ...sets.slice(1)]
	return [
		{ at, mods },
		...sets.filter((set) => signature(set.mods) !== signature(mods)),
	].slice(0, KEPT)
}

/**
 * A remembered set as a draft for this room: whatever the room already has,
 * or its host offers, under the name the host knows it by.
 */
export function adopt(
	mods: readonly Pick[],
	room: readonly Pick[],
	offered: readonly Offer[],
): Pick[] {
	return mods.map((mod) => {
		const held = room.find((pick) => pick.source && pick.source === mod.source)
		if (held) return held
		const offer = offered.find(
			(entry) => entry.source && entry.source === mod.source,
		)
		return offer ? { ...mod, ref: offer.name } : mod
	})
}

/** The sets kept in the browser's storage, or none when there are none to trust. */
export function readSets(storage: Storage | null): ModSet[] {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(SETS_KEY) ?? '[]')
		if (!Array.isArray(parsed)) return []
		return parsed.filter(isSet)
	} catch {
		return []
	}
}

export function writeSets(storage: Storage | null, sets: readonly ModSet[]) {
	try {
		storage?.setItem(SETS_KEY, JSON.stringify(sets))
	} catch {
		// Storage that is full or off loses the history, nothing more.
	}
}

export const SETS_KEY = 'modlobby.modSets'

function isSet(value: unknown): value is ModSet {
	if (typeof value !== 'object' || value === null) return false
	const { at, mods } = value as { at?: unknown; mods?: unknown }
	return (
		typeof at === 'string' &&
		Array.isArray(mods) &&
		mods.length > 0 &&
		mods.every(
			(mod: unknown) =>
				typeof mod === 'object' &&
				mod !== null &&
				typeof (mod as Pick).ref === 'string' &&
				typeof (mod as Pick).label === 'string',
		)
	)
}
