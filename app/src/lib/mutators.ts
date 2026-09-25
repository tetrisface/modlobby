import type { MutatorView } from '../ipc/bindings/MutatorView'

/**
 * Where a mutator comes from, for a person to read: `owner/repo @ 9108a17`
 * out of the host's `github:owner/repo@<commit>`. Anything else is shown as
 * it came.
 */
export function sourceWords(source: string): string {
	const [, repo, commit] = /^github:([^@]+)@([0-9a-f]{40})$/.exec(source) ?? []
	return repo && commit ? `${repo} @ ${commit.slice(0, 7)}` : source
}

/** A mutator the room's host offers to load, by the name it is added by. */
export type Offer = {
	name: string
	source: string | null
	date: string | null
}

/**
 * What the room's host offers, as its script tags list it: `game/mutatoroffer0`
 * onwards, to the first gap. A room whose host runs no mutators has none.
 */
export function offers(tags: { [key in string]: string } | undefined): Offer[] {
	const found: Offer[] = []
	for (let index = 0; ; index++) {
		const key = `game/mutatoroffer${index}`
		const name = tags?.[key]
		if (!name) return found
		found.push({
			name,
			source: tags[`${key}source`] ?? null,
			date: tags[`${key}date`] ?? null,
		})
	}
}

/**
 * Whether the room's host runs mods: it says so, or offers or loads some.
 * The first alone is enough, so a host with nothing loaded or used yet still
 * shows where mods are added.
 */
export function hostsMods(
	tags: { [key in string]: string } | undefined,
): boolean {
	return Boolean(tags?.['game/mutatorhost']) || offers(tags).length > 0
}

/**
 * Whether a loaded mutator is this offer: one from GitHub by the commit it is
 * built from, which the room and the offer share after an update too; any
 * other by its name.
 */
export function sameMutator(mutator: MutatorView, offer: Offer): boolean {
	return offer.source
		? mutator.source === offer.source
		: mutator.name === offer.name
}
