import type { ModOption } from '../ipc/bindings/ModOption'

/**
 * A stand-in for BAR's modoption table.
 *
 * The real table is not in this repository. Its option descriptions are BAR's
 * writing under GPL v2, so the app reads `modoptions.lua` out of the copy the
 * player already has rather than shipping one — see `lib/setup`. That leaves
 * the arrangement rules with nothing to be tested against, hence this.
 *
 * What it reproduces is structure, not prose: section keys, their weights, the
 * subheaders that become groups, and the handful of option keys the rules
 * actually name. Those are interface identifiers rather than authored text,
 * and they are what the rules key off. Every description here is invented.
 *
 * Whether the real table still looks like this is a different question, and
 * one for a check against a real install rather than for a unit test:
 * `cargo run -p content --example read_game_file`.
 */

type Spec = {
  key: string
  type?: string
  name?: string
  section?: string
  weight?: number
  hidden?: boolean
  def?: string
  items?: Array<{ key: string; name: string }>
}

const option = (spec: Spec): ModOption =>
  ({
    key: spec.key,
    name: spec.name ?? spec.key,
    desc: '',
    type: spec.type ?? 'bool',
    def: spec.def ?? '',
    items: spec.items,
    section: spec.section,
    weight: spec.weight,
    hidden: spec.hidden,
  }) as unknown as ModOption

/** Sections in BAR's declared weight order, heaviest first. */
const SECTIONS: Array<[string, string, number]> = [
  ['options_main', 'Main', 100],
  ['raptor_defense_options', 'Raptors', 90],
  ['scavengers', 'Scavengers', 80],
  ['extras', 'Extras', 70],
  ['experimental', 'Experimental', 60],
  ['other', 'Other', 0],
  ['options_cheats', 'Cheats', -10],
  // Chobby drops this one outright, and so do we.
  ['dev', 'Dev', -20],
]

/** Cheats' own subheaders, and enough options under each to be realistic. */
const CHEAT_GROUPS: Array<[string, string[]]> = [
  ['AI Cheats', ['dynamiccheats', 'aicheats_resources']],
  [
    'Starting Resources',
    ['startmetal', 'startenergy', 'startmetalstorage', 'startenergystorage'],
  ],
  [
    'Resource Multipliers',
    [
      'multiplier_resourceincome',
      'multiplier_metalcost',
      'multiplier_energycost',
      'multiplier_buildpower',
      'multiplier_buildtimecost',
      'nowasting',
    ],
  ],
  [
    'Unit Parameter Multipliers',
    [
      'multiplier_maxdamage',
      'multiplier_turnrate',
      'multiplier_losrange',
      'multiplier_radarrange',
      'multiplier_weaponrange',
      'multiplier_weapondamage',
      'multiplier_weaponreload',
      'multiplier_shieldpower',
      'multiplier_maxvelocity',
      'multiplier_buildrange',
    ],
  ],
]

/**
 * Types and defaults for the few keys the rules are tested against.
 *
 * A rule that decides whether a value counts as changed needs something to
 * compare against; everything else can stay a bool with no default.
 */
const DEFAULTS: Record<string, { type: string; def: string }> = {
  dynamiccheats: { type: 'bool', def: '1' },
  startmetal: { type: 'number', def: '1000' },
  startenergy: { type: 'number', def: '1000' },
  multiplier_buildpower: { type: 'number', def: '1' },
  // A list, so the rule that shows an item's name rather than its key has
  // something to show.
  nowasting: { type: 'list', def: 'enabled' },
}

/** Items for the one list option above. */
const LIST_ITEMS = [
  { key: 'enabled', name: 'Enabled' },
  { key: 'disabled', name: 'Disabled' },
]

export function fixtureOptions(): ModOption[] {
  const options: ModOption[] = []

  for (const [key, name, weight] of SECTIONS) {
    options.push(option({ key, name, type: 'section', weight }))
  }

  // One ordinary option per section, so none of them is empty.
  for (const [key] of SECTIONS) {
    if (key === 'options_cheats') continue
    options.push(option({ key: `${key}_setting`, section: key }))
  }

  for (const [label, keys] of CHEAT_GROUPS) {
    options.push(
      option({
        key: `sub_${label}`,
        type: 'subheader',
        name: `-- ${label}`,
        section: 'options_cheats',
      }),
    )
    for (const key of keys) {
      options.push(
        option({
          key,
          section: 'options_cheats',
          type: DEFAULTS[key]?.type,
          def: DEFAULTS[key]?.def,
          items: DEFAULTS[key]?.type === 'list' ? LIST_ITEMS : undefined,
        }),
      )
    }
  }

  // BAR's trailing group under Cheats holds only options that are hidden or
  // that move to Modding, so it empties out and stops being drawn.
  options.push(
    option({
      key: 'sub_other',
      type: 'subheader',
      name: '-- Other',
      section: 'options_cheats',
    }),
  )
  options.push(
    option({
      key: 'experimentalshields',
      section: 'options_cheats',
      hidden: true,
    }),
  )
  options.push(
    option({ key: 'holiday_events', section: 'options_cheats', hidden: true }),
  )

  // The six that are lifted into the Modding tab, in the sections BAR puts
  // them in.
  for (const [key, section] of [
    ['tweakdefs', 'options_cheats'],
    ['tweakunits', 'options_cheats'],
    ['forceallunits', 'options_cheats'],
    ['experimentallegionfaction', 'experimental'],
    ['experimentalextraunits', 'experimental'],
    ['scavunitsforplayers', 'scavengers'],
  ] as const) {
    options.push(option({ key, section }))
  }

  return options
}
