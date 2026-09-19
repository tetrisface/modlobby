/**
 * Chobby's search rule: every whitespace-separated word of the query is a
 * substring of the text, in any order and any case
 * (`battle_list_window.lua:803-845`). An empty query matches everything.
 */
export function hasEveryWord(text: string, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const haystack = text.toLowerCase()
  return words.every((word) => haystack.includes(word))
}
