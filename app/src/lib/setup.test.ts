import { describe, expect, test } from 'bun:test'
import {
  MODDING_TAB,
  TWEAK_SLOTS,
  changedCount,
  defaultText,
  displayText,
  readModOptions,
  rowsOf,
  tabs,
  type Tab,
} from './setup'

const TABS = tabs()
const byKey = (key: string): Tab => {
  const tab = TABS.find((entry) => entry.key === key)
  if (!tab) throw new Error(`no tab ${key}`)
  return tab
}
const optionKeys = (tab: Tab) =>
  tab.groups.flatMap((group) => group.options.map((option) => option.key))

describe('tabs', () => {
  test('are BAR sections by weight, with Modding last', () => {
    expect(TABS.map((tab) => tab.name)).toEqual([
      'Main',
      'Raptors',
      'Scavengers',
      'Extras',
      'Experimental',
      'Other',
      'Cheats',
      'Modding',
    ])
  })

  test('Cheats keeps its name and its balance settings', () => {
    const cheats = byKey('options_cheats')
    const keys = optionKeys(cheats)
    expect(keys).toContain('startmetal')
    expect(keys).toContain('multiplier_buildpower')
    expect(keys).toContain('dynamiccheats')
    expect(keys.length).toBeGreaterThan(20)
    // `experimentalshields` and `holiday_events` are declared hidden in BAR,
    // so Chobby draws neither and neither do we.
    expect(keys).not.toContain('experimentalshields')
  })

  test('groups inside Cheats are BAR own subheaders', () => {
    // BAR's trailing `-- Other` group held only the two hidden options and the
    // four that move to Modding, so it empties out and stops being drawn.
    expect(byKey('options_cheats').groups.map((group) => group.name)).toEqual([
      'AI Cheats',
      'Starting Resources',
      'Resource Multipliers',
      'Unit Parameter Multipliers',
    ])
  })

  test('every modding option leaves its old tab exactly once', () => {
    const moved = [
      'tweakdefs',
      'tweakunits',
      'forceallunits',
      'experimentallegionfaction',
      'experimentalextraunits',
      'scavunitsforplayers',
    ]
    const modding = optionKeys(byKey(MODDING_TAB))
    for (const key of moved) {
      expect(modding).toContain(key)
      const elsewhere = TABS.filter((tab) => tab.key !== MODDING_TAB).filter(
        (tab) => optionKeys(tab).includes(key),
      )
      expect(elsewhere.map((tab) => tab.name)).toEqual([])
    }
  })

  test('tweak slots are all twenty, defs before units', () => {
    expect(TWEAK_SLOTS).toHaveLength(20)
    expect(TWEAK_SLOTS[0]).toBe('tweakdefs')
    expect(TWEAK_SLOTS[9]).toBe('tweakdefs9')
    expect(TWEAK_SLOTS[10]).toBe('tweakunits')
    expect(TWEAK_SLOTS[19]).toBe('tweakunits9')
  })

  test('hidden options never become a row', () => {
    // `holiday_events` is declared hidden in the Cheats section.
    expect(optionKeys(byKey('options_cheats'))).not.toContain('holiday_events')
  })
})

describe('changes against BAR defaults', () => {
  const values = readModOptions({
    'game/modoptions/startmetal': '2000',
    'game/modoptions/multiplier_buildpower': '1.5',
    'game/modoptions/startenergy': '1000',
    'game/modoptions/dynamiccheats': '1',
    'game/modoptions/tweakdefs1': 'bG9jYWw=',
    'game/hosttype': 'SPADS',
    'game/players/bob/skill': '[14]',
  })

  test('reads only the modoption tags', () => {
    expect(values).toEqual({
      startmetal: '2000',
      multiplier_buildpower: '1.5',
      startenergy: '1000',
      dynamiccheats: '1',
      tweakdefs1: 'bG9jYWw=',
    })
  })

  test('a value equal to the default is not a change', () => {
    // startenergy's default is 1000, and dynamiccheats defaults to true.
    const cheats = byKey('options_cheats')
    const changed = cheats.groups
      .flatMap((group) => rowsOf(group, values))
      .filter((row) => row.changed)
      .map((row) => row.option.key)
    expect(changed.sort()).toEqual(['multiplier_buildpower', 'startmetal'])
  })

  test('a number compares numerically, not as text', () => {
    const rows = rowsOf(
      byKey('options_cheats').groups.find(
        (group) => group.name === 'Starting Resources',
      )!,
      readModOptions({ 'game/modoptions/startmetal': '1000.0' }),
    )
    const metal = rows.find((row) => row.option.key === 'startmetal')
    expect(metal?.changed).toBe(false)
  })

  test('the tab badge counts what the tab shows', () => {
    expect(changedCount(byKey('options_cheats'), values)).toBe(2)
    expect(changedCount(byKey('raptor_defense_options'), values)).toBe(0)
  })
})

describe('display', () => {
  const cheats = byKey('options_cheats')
  const row = (key: string, values: Record<string, string>) =>
    cheats.groups
      .flatMap((group) => rowsOf(group, values))
      .find((entry) => entry.option.key === key)!

  test('a bool reads as on or off, however it arrived', () => {
    expect(displayText(row('dynamiccheats', { dynamiccheats: '0' }))).toBe(
      'off',
    )
    expect(displayText(row('dynamiccheats', { dynamiccheats: 'true' }))).toBe(
      'on',
    )
  })

  test('a list shows the item name, not its key', () => {
    expect(displayText(row('nowasting', { nowasting: 'disabled' }))).toBe(
      'Disabled',
    )
  })

  test('an unset row falls back to the default it sits on', () => {
    expect(displayText(row('startmetal', {}))).toBe('1000')
    expect(defaultText(row('dynamiccheats', {}).option)).toBe('1')
  })
})
