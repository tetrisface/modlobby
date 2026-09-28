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
