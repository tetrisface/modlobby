import type { Preset } from '../ipc/bindings/Preset'

/**
 * Ordering and describing saved room setups.
 *
 * Pure, so the awkward parts — a preset never used, two used in the same
 * second — are settled in tests rather than in a table component.
 */

export type Column = 'name' | 'map' | 'options' | 'created' | 'updated' | 'used'

/** Last used first: the one you want is nearly always the one you just had. */
export const DEFAULT_SORT: Column = 'used'

/** Which way round a column reads when you first click it. */
export function naturalDescending(column: Column): boolean {
  // Dates and sizes are most interesting at the top; names read A to Z.
  return column !== 'name' && column !== 'map'
}

export function search(presets: Preset[], needle: string): Preset[] {
  const words = needle.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return presets
  return presets.filter((preset) => {
    const haystack = `${preset.name} ${preset.map ?? ''}`.toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
}

function value(preset: Preset, column: Column): string | number {
  switch (column) {
    case 'name':
      return preset.name.toLowerCase()
    case 'map':
      return (preset.map ?? '').toLowerCase()
    case 'options':
      return Object.keys(preset.modoptions).length
    case 'created':
      return preset.created
    case 'updated':
      return preset.updated
    case 'used':
      // Never used sorts last however the column is pointed, because "no date"
      // is not an early date — it is the absence of one.
      return preset.lastUsed ?? -1
  }
}

export function sort(
  presets: Preset[],
  column: Column,
  descending: boolean,
): Preset[] {
  return [...presets].sort((a, b) => {
    const left = value(a, column)
    const right = value(b, column)
    let order = 0
    if (typeof left === 'string' && typeof right === 'string')
      order = left.localeCompare(right)
    else order = Number(left) - Number(right)
    // A stable tiebreak, so a column of equal dates does not shuffle about
    // every time the table redraws.
    if (order === 0) return a.name.localeCompare(b.name)
    return descending ? -order : order
  })
}

/** How many tweak slots a preset actually fills. */
export function tweakCount(preset: Preset): number {
  return Object.entries(preset.modoptions).filter(
    ([key, value]) =>
      (key.startsWith('tweakdefs') || key.startsWith('tweakunits')) &&
      value.length > 1,
  ).length
}

/** A date as a person reads it in a table: recent things in relative terms. */
export function when(stamp: number | null, now: number): string {
  if (stamp === null) return 'never'
  const seconds = Math.max(0, Math.floor(now / 1000) - stamp)
  if (seconds < 90) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 14) return `${days}d ago`
  return new Date(stamp * 1000).toISOString().slice(0, 10)
}
