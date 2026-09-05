/**
 * BAR's modoptions, arranged into the tabs a room shows.
 *
 * The table is the game's own `modoptions.lua`, read out of the copy already
 * installed on this machine and parsed by the `modoptions` crate. It is not
 * shipped with the app: the descriptions in it are BAR's writing under GPL v2,
 * and a lobby has no need to redistribute them when every player already has
 * the file. bar-lobby reads it the same way (`game-provider.ts:200`), and
 * Chobby asks the engine's Lua VM for it.
 *
 * The happy side effect is that the table can never be out of date with the
 * game a room is actually running.
 *
 * Tab order, group names and defaults are all BAR's own — the one thing we
 * impose is the Modding tab.
 */

import type { ModOption } from '../ipc/bindings/ModOption'
import type { OptionValue } from '../ipc/bindings/OptionValue'

export type Group = { name: string; options: ModOption[] }
export type Tab = { key: string; name: string; desc: string; groups: Group[] }

export type Row = {
  option: ModOption
  /** What the room has set, if anything. */
  current: string | null
  changed: boolean
}

/** Chobby nulls this section outright (`gui_modoptions_panel.lua:1242`). */
const DROPPED_SECTION = 'dev'

/**
 * The six options we lift into a Modding tab, with the groups they land in.
 *
 * They are one mechanism: each decides which unit definitions exist. BAR's own
 * `forceallunits` description — "Load all UnitDefs even if ais or options for
 * them aren't enabled" — exists to serve the other five and the tweak slots.
 * `section` is only a lobby display hint ("so lobbies can order options in
 * categories/panels"), so moving them changes nothing on the wire.
 */
const MODDING_GROUPS: ReadonlyArray<readonly [string, readonly string[]]> = [
  [
    'Unit packs',
    [
      'experimentallegionfaction',
      'experimentalextraunits',
      'scavunitsforplayers',
    ],
  ],
  ['Loading', ['forceallunits']],
]

/**
 * The two slots BAR declares by hand, both in Cheats. Slots 1-9 come from the
 * `for` loops the vendoring step does not read, so they appear nowhere else.
 */
const DECLARED_TWEAK_SLOTS = ['tweakdefs', 'tweakunits']

const MOVED = new Set([
  ...MODDING_GROUPS.flatMap(([, keys]) => keys),
  ...DECLARED_TWEAK_SLOTS,
])

/** The 20 slots the `tweaks` crate models; BAR declares 1-9 as hidden. */
export const TWEAK_SLOTS: readonly string[] = ['defs', 'units'].flatMap(
  (kind) =>
    ['', '1', '2', '3', '4', '5', '6', '7', '8', '9'].map(
      (index) => `tweak${kind}${index}`,
    ),
)

export const MODDING_TAB = 'modding'

/**
 * Not one of BAR's tabs: the one that shows what is changed in all of them.
 * A room's four changed settings are spread over three tabs, and finding
 * them was a tour.
 */
export const ALL_TAB = 'all'

/**
 * Groups come from BAR's own `-- Name` subheaders. Plain subheaders are prose
 * for the tab, separators are spacing, and both are layout rather than
 * settings, so neither becomes a row.
 */
function groupsOf(options: ModOption[]): Group[] {
  const groups: Group[] = []
  let current: Group = { name: '', options: [] }

  for (const option of options) {
    if (option.type === 'separator') continue
    if (option.type === 'subheader') {
      const label = option.name ?? ''
      if (!label.startsWith('--')) continue
      if (current.options.length > 0) groups.push(current)
      current = { name: label.replace(/^--\s*/, '').trim(), options: [] }
      continue
    }
    if (option.hidden) continue
    current.options.push(option)
  }

  if (current.options.length > 0) groups.push(current)
  return groups
}

function moddingTab(options: ModOption[]): Tab {
  const byKey = new Map(options.map((option) => [option.key, option]))
  const groups: Group[] = [
    {
      name: 'Tweak slots',
      options: TWEAK_SLOTS.map((key) => ({
        key,
        name: key,
        desc: 'Base64url Lua carried as a modoption.',
        type: 'string',
        def: '',
      })),
    },
  ]

  for (const [name, keys] of MODDING_GROUPS) {
    const options = keys
      .map((key) => byKey.get(key))
      .filter((option): option is ModOption => option !== undefined)
    if (options.length > 0) groups.push({ name, options })
  }

  return {
    key: MODDING_TAB,
    name: 'Modding',
    desc: 'What unit definitions the game loads, and the Lua that rewrites them.',
    groups,
  }
}

/**
 * Tabs in Chobby's order: weight descending, with an unweighted section
 * treated as zero so it lands between Experimental and Cheats. Modding goes
 * last, next to Cheats, where the tweak slots used to live.
 */
export function tabs(options: ModOption[]): Tab[] {
  const sections = options
    .filter(
      (option) =>
        option.type === 'section' &&
        !option.hidden &&
        option.key !== DROPPED_SECTION,
    )
    .sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0))

  const declared = sections.map((section) => ({
    key: section.key,
    name: section.name ?? section.key,
    desc: section.desc ?? '',
    groups: groupsOf(
      options.filter(
        (option) => option.section === section.key && !MOVED.has(option.key),
      ),
    ),
  }))

  return [...declared, moddingTab(options)]
}

/** Modoptions the room has set, keyed without the `game/modoptions/` prefix. */
export function readModOptions(
  scriptTags: Record<string, string> | undefined,
): Record<string, string> {
  const values: Record<string, string> = {}
  if (!scriptTags) return values

  for (const [key, value] of Object.entries(scriptTags)) {
    const name = /^game\/modoptions\/(.+)$/.exec(key)?.[1]
    if (name !== undefined) values[name] = value
  }
  return values
}

/** How Lua's default reads once it has been through the protocol. */
export function defaultText(option: ModOption): string {
  const def: OptionValue | null | undefined = option.def
  if (def === null || def === undefined) return ''
  if (typeof def === 'boolean') return def ? '1' : '0'
  return String(def)
}

export function isOn(text: string): boolean {
  return text === '1' || text.toLowerCase() === 'true'
}

function isChanged(option: ModOption, current: string): boolean {
  const def = defaultText(option)
  if (option.type === 'number') return Number(current) !== Number(def)
  if (option.type === 'bool') return isOn(current) !== isOn(def)
  return current !== def
}

/** What a row is called: BAR's name, or the key when it has none. */
export function label(option: ModOption): string {
  return option.name || option.key
}

export function rowsOf(group: Group, values: Record<string, string>): Row[] {
  return group.options.map((option) => {
    const current = values[option.key] ?? null
    return {
      option,
      current,
      changed: current !== null && isChanged(option, current),
    }
  })
}

export type Changed = { tab: Tab; rows: Row[] }

/**
 * Every setting by the tab it lives in -- only what differs from BAR's
 * default, or all of it. Tabs with nothing to show are left out.
 */
export function rowsByTab(
  tabs: Tab[],
  values: Record<string, string>,
  onlyChanged: boolean,
): Changed[] {
  return tabs
    .map((tab) => ({
      tab,
      rows: tab.groups
        .flatMap((group) => rowsOf(group, values))
        .filter((row) => row.changed || !onlyChanged),
    }))
    .filter((entry) => entry.rows.length > 0)
}

/**
 * Every setting whose name, key, description or value carries every word of
 * the needle, by the tab it lives in. Nothing for an empty needle: that is
 * the tabs' job. Tweak slots have only a key, which is what people type for
 * them; their blob is base64 and stays out of it.
 */
export function searchRows(
  tabs: Tab[],
  values: Record<string, string>,
  needle: string,
): Changed[] {
  const words = needle.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return []
  return rowsByTab(tabs, values, false)
    .map((entry) => ({
      tab: entry.tab,
      rows: entry.rows.filter((row) => {
        const haystack = searchText(row).toLowerCase()
        return words.every((word) => haystack.includes(word))
      }),
    }))
    .filter((entry) => entry.rows.length > 0)
}

/**
 * What a row can be found by: the value as the row shows it -- `on`, an
 * item's name, the number -- and as the room holds it, so a list item's key
 * and `true` count too.
 */
function searchText(row: Row): string {
  const { option } = row
  const value = isTweakSlot(row)
    ? ''
    : `${displayText(row)} ${row.current ?? ''}`
  return `${label(option)} ${option.key} ${option.desc ?? ''} ${value}`
}

/** Every setting that differs from BAR's default, by the tab it lives in. */
export function changedByTab(
  tabs: Tab[],
  values: Record<string, string>,
): Changed[] {
  return rowsByTab(tabs, values, true)
}

/** Whether a row is one of the twenty tweak slots, drawn as actions, not a value. */
export function isTweakSlot(row: Row): boolean {
  return TWEAK_SLOTS.includes(row.option.key)
}

/** How many of a tab's settings differ from BAR's default. */
export function changedCount(tab: Tab, values: Record<string, string>): number {
  return tab.groups.reduce(
    (total, group) =>
      total + rowsOf(group, values).filter((row) => row.changed).length,
    0,
  )
}

/** What a row shows on the right: the value, or the default it is sitting on. */
export function displayText(row: Row): string {
  const text = row.current ?? defaultText(row.option)
  // A tweak is a blob of base64; its size is the one thing a row can say.
  if (TWEAK_SLOTS.includes(row.option.key))
    return text === '' ? 'empty' : `${text.length} B`
  if (row.option.type === 'bool') return isOn(text) ? 'on' : 'off'
  if (row.option.type === 'string' && text === '') return 'empty'
  const item = row.option.items?.find((entry) => entry.key === text)
  return item?.name ?? text
}
