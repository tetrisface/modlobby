/**
 * What the game and the engine know, offered while a tweak is written.
 *
 * Two vocabularies: the units a game has -- the keys a `tweakunits` table
 * may use, and what `unitdefs_post.lua` silently ignores when they are
 * misspelt -- and the weapon tags the engine reads under `weapondefs`. The
 * engine keeps its unit tags in hand-written parsing rather than a registry,
 * so there is no third list; see `crates/tweaks/src/assist.rs`.
 *
 * Pure: the providers in `views/tweaks/providers.ts` call these with the text
 * around the cursor and draw what comes back.
 */

import type { Symbol } from '../ipc/bindings/Symbol'
import type { Tag } from '../ipc/bindings/Tag'

export type Assist = {
  /** Lowercased unit names, as the game's `units/` folder spells them. */
  units: string[]
  weaponTags: Tag[]
}

export const NO_ASSIST: Assist = { units: [], weaponTags: [] }

/**
 * The keys of the tables open at `offset`, outermost first.
 *
 * `{ corgolt4 = { weapondefs = { corgol_sidelaser = { dam` gives
 * `['corgolt4', 'weapondefs', 'corgol_sidelaser']`. A table opened without
 * a key -- an array element, or the root -- contributes `''`. Counts braces
 * outside strings and comments, which is enough for a table constructor;
 * a `tweakdefs` script with braces in code gets an answer that is merely
 * wrong, and the callers treat one they cannot place as no context.
 */
export function pathAt(text: string, offset: number): string[] {
  const path: string[] = []
  let i = 0
  const end = Math.min(offset, text.length)
  while (i < end) {
    const ch = text[i]!
    if (ch === '-' && text[i + 1] === '-') {
      const eol = text.indexOf('\n', i)
      i = eol === -1 ? end : eol + 1
      continue
    }
    if (ch === '"' || ch === "'") {
      i = skipString(text, i, end)
      continue
    }
    if (ch === '{') {
      path.push(keyBefore(text, i))
      i += 1
      continue
    }
    if (ch === '}') {
      path.pop()
      i += 1
      continue
    }
    i += 1
  }
  return path
}

/** Past a quoted string that opens at `at`, honouring backslashes. */
function skipString(text: string, at: number, end: number): number {
  const quote = text[at]
  let i = at + 1
  while (i < end) {
    if (text[i] === '\\') {
      i += 2
      continue
    }
    if (text[i] === quote) return i + 1
    i += 1
  }
  return end
}

/** The `name =` or `["name"] =` immediately before a `{`, or `''`. */
function keyBefore(text: string, brace: number): string {
  const before = text.slice(Math.max(0, brace - 200), brace)
  const match =
    /(?:\[\s*["']([^"']+)["']\s*\]|([A-Za-z_][A-Za-z0-9_]*))\s*=\s*$/.exec(
      before,
    )
  return match?.[1] ?? match?.[2] ?? ''
}

export type Suggestion = { name: string; detail: string; doc?: string }

/**
 * What may be typed where the cursor is, in a `tweakunits` table.
 *
 * Directly inside the root table, a unit; directly inside a weapon's table
 * (a table under `weapondefs`), a weapon tag. Anywhere else, nothing --
 * unit tags have no registry to draw from, and guessing is worse than
 * silence in a completion list.
 */
export function suggestions(path: string[], assist: Assist): Suggestion[] {
  if (path.length === 1) {
    return assist.units.map((name) => ({ name, detail: 'unit' }))
  }
  if (path.length >= 2 && path[path.length - 2] === 'weapondefs') {
    return assist.weaponTags.map((tag) => ({
      name: tag.name,
      detail: describeTag(tag),
      doc: tag.description ?? undefined,
    }))
  }
  return []
}

/** A tag's type and default in a few characters: `float = 1.0`. */
export function describeTag(tag: Tag): string {
  return tag.default === null ? tag.kind : `${tag.kind} = ${tag.default}`
}

/**
 * The tag under a word, when the word is a weapon tag in a weapon's table.
 * Weapon tags are matched without case, as the engine reads them.
 */
export function tagAt(
  path: string[],
  word: string,
  assist: Assist,
): Tag | null {
  if (path.length < 2 || path[path.length - 2] !== 'weapondefs') return null
  const wanted = word.toLowerCase()
  return (
    assist.weaponTags.find((tag) => tag.name.toLowerCase() === wanted) ?? null
  )
}

export type Warning = { line: number; message: string }

/**
 * Unit keys the game does not have. `unitdefs_post.lua` merges a tweak by
 * looking each key up in `UnitDefs` and skips what it does not find, so a
 * misspelt unit is a tweak that quietly does nothing.
 */
export function unknownUnits(outline: Symbol[], units: string[]): Warning[] {
  if (units.length === 0) return []
  const known = new Set(units)
  return outline
    .filter((symbol) => !known.has(symbol.name.toLowerCase()))
    .map((symbol) => ({
      line: symbol.line,
      message: `no unit named ${symbol.name} in this game; the tweak skips it`,
    }))
}
