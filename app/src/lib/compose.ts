/**
 * Writing a line: recalling the last one, and finishing a name.
 *
 * Both are things every lobby has had for twenty years, and both are worse to
 * live without here than elsewhere: the lines people repeat in a BAR room are
 * SPADS commands like `!bSet tweakdefs1 …`, and the names they address are
 * `[Crd]XxStormKittyxX`.
 *
 * Everything here is a plain function over strings so the behaviour can be
 * tested without a keyboard.
 */

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

/** The word the caret sits in, and where it starts. */
export function wordAt(
  text: string,
  caret: number,
): { word: string; from: number } {
  const before = text.slice(0, caret)
  const from = before.lastIndexOf(' ') + 1
  return { word: before.slice(from), from }
}

/**
 * Names that could finish a partial one, in the order they should be offered.
 *
 * Case-insensitive, because nobody types a name the way it was registered. A
 * name that starts with what was typed comes before one that merely contains
 * it: typing `sky` means `Skywalker` far more often than `BlueSky`.
 */
export function completions(word: string, names: string[]): string[] {
  const needle = word.toLowerCase()
  if (!needle) return []
  const starts: string[] = []
  const contains: string[] = []
  for (const name of names) {
    const lower = name.toLowerCase()
    if (lower === needle) continue
    if (lower.startsWith(needle)) starts.push(name)
    else if (lower.includes(needle)) contains.push(name)
  }
  starts.sort((a, b) => a.localeCompare(b))
  contains.sort((a, b) => a.localeCompare(b))
  return [...starts, ...contains]
}

/**
 * The line with the partial name replaced by a whole one.
 *
 * A completion at the very start of the line is addressing somebody, so it
 * gets the colon that a lobby reads as "this is for you"; anywhere else it is
 * just a name in a sentence and gets a space.
 */
export function complete(
  text: string,
  caret: number,
  name: string,
): { text: string; caret: number } {
  const { from } = wordAt(text, caret)
  const tail = from === 0 ? `${name}: ` : `${name} `
  const next = text.slice(0, from) + tail + text.slice(caret)
  return { text: next, caret: from + tail.length }
}
