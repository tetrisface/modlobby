import { describe, expect, test } from 'vitest'
import type { Tag } from '../ipc/bindings/Tag'
import {
  describeTag,
  pathAt,
  suggestions,
  tagAt,
  unknownUnits,
  type Assist,
} from './assist'

const TWEAK = `-- Golem { not a brace }
{
\tcorgolt4 = {
\t\tbuildpic = "scav/{corgolt4}.dds",
\t\tweapondefs = {
\t\t\tcorgol_sidelaser = { damage = { default = 7250 }, range = 1075 },
\t\t\t["cor_laser"] = { ra
\t\t\t},
\t\t},
\t},
\tarmcom = {}
}`

const at = (needle: string) => TWEAK.indexOf(needle) + needle.length

describe('pathAt', () => {
  test('names the tables open at the cursor, outermost first', () => {
    expect(pathAt(TWEAK, at('corgolt4 = {'))).toEqual(['', 'corgolt4'])
    expect(pathAt(TWEAK, at('range = 1075'))).toEqual([
      '',
      'corgolt4',
      'weapondefs',
      'corgol_sidelaser',
    ])
    expect(pathAt(TWEAK, at('["cor_laser"] = { ra'))).toEqual([
      '',
      'corgolt4',
      'weapondefs',
      'cor_laser',
    ])
    expect(pathAt(TWEAK, at('default = 7250'))).toEqual([
      '',
      'corgolt4',
      'weapondefs',
      'corgol_sidelaser',
      'damage',
    ])
  })

  test('braces in comments and strings do not count', () => {
    expect(pathAt(TWEAK, at('buildpic = "scav/{corgolt4}.dds",'))).toEqual([
      '',
      'corgolt4',
    ])
  })

  test('a closed table is left again', () => {
    expect(pathAt(TWEAK, at('armcom = {}'))).toEqual([''])
    expect(pathAt(TWEAK, TWEAK.length)).toEqual([])
  })
})

const tag = (over: Partial<Tag>): Tag => ({
  name: 'range',
  kind: 'float',
  default: '0',
  description: 'How far it shoots.',
  min: null,
  max: null,
  ...over,
})

const assist: Assist = {
  units: ['armcom', 'corgolt4'],
  weaponTags: [
    tag({}),
    tag({ name: 'reloadTime', default: '1', kind: 'float' }),
  ],
}

describe('suggestions', () => {
  test('units at the top, weapon tags in a weapon, nothing elsewhere', () => {
    expect(suggestions([''], assist).map((s) => s.name)).toEqual([
      'armcom',
      'corgolt4',
    ])
    expect(
      suggestions(['', 'corgolt4', 'weapondefs', 'x'], assist).map(
        (s) => s.name,
      ),
    ).toEqual(['range', 'reloadTime'])
    expect(suggestions(['', 'corgolt4'], assist)).toEqual([])
    expect(suggestions(['', 'corgolt4', 'weapondefs'], assist)).toEqual([])
    expect(suggestions([], assist)).toEqual([])
  })

  test('a tag is described by type and default', () => {
    expect(describeTag(tag({}))).toBe('float = 0')
    expect(describeTag(tag({ default: null, kind: 'table' }))).toBe('table')
  })
})

describe('tagAt', () => {
  test('finds a weapon tag under the cursor regardless of case', () => {
    expect(
      tagAt(['', 'u', 'weapondefs', 'w'], 'RELOADTIME', assist)?.name,
    ).toBe('reloadTime')
    expect(tagAt(['', 'u'], 'range', assist)).toBeNull()
    expect(tagAt(['', 'u', 'weapondefs', 'w'], 'nosuch', assist)).toBeNull()
  })
})

describe('unknownUnits', () => {
  test('warns about a unit the game does not have, and stays quiet without a list', () => {
    const outline = [
      { name: 'armcom', line: 2 },
      { name: 'CorGolt4', line: 9 },
      { name: 'armcomm', line: 30 },
    ]
    expect(unknownUnits(outline, ['armcom', 'corgolt4'])).toEqual([
      {
        line: 30,
        message: 'no unit named armcomm in this game; the tweak skips it',
      },
    ])
    expect(unknownUnits(outline, [])).toEqual([])
  })
})
