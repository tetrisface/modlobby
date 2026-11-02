/**
 * Chobby's search rule: every whitespace-separated word of the query is a
 * substring of the text, in any order and any case
 * (`battle_list_window.lua:803-845`). An empty query matches everything.
 *
 * A caller matching many texts against one query splits it once with
 * `wordsOf` and passes the words.
 */
export function hasEveryWord(
	text: string,
	query: string | readonly string[],
): boolean {
	const words = typeof query === 'string' ? wordsOf(query) : query
	const haystack = text.toLowerCase()
	return words.every((word) => haystack.includes(word))
}

/** The query as lowercased words. */
export const wordsOf = (query: string): string[] =>
	query.toLowerCase().split(/\s+/).filter(Boolean)

export type Piece = { text: string; hit: boolean }

/**
 * The text cut up by what a query finds in it, for marking the hits. A letter
 * that changes length when folded would put the hits in the wrong place, so a
 * text holding one is left whole.
 */
export function pieces(text: string, query: string): Piece[] {
	const words = wordsOf(query)
	const folded = text.toLowerCase()
	if (words.length === 0 || folded.length !== text.length)
		return [{ text, hit: false }]

	const found = new Array<boolean>(text.length).fill(false)
	for (const word of words) {
		for (
			let at = folded.indexOf(word);
			at !== -1;
			at = folded.indexOf(word, at + 1)
		) {
			found.fill(true, at, at + word.length)
		}
	}

	const cut: Piece[] = []
	for (let start = 0, end = 1; start < text.length; end++) {
		if (end < text.length && found[end] === found[start]) continue
		cut.push({ text: text.slice(start, end), hit: found[start] === true })
		start = end
	}
	return cut
}
