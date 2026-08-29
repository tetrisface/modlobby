import { describe, expect, test } from 'bun:test'
import {
  isUnrated,
  parseSkill,
  readSkills,
  skillText,
  skillTier,
  teamSkill,
} from './skill'

describe('parseSkill', () => {
  test('reads every sigil SPADS can send', () => {
    // sendPlayerSkill, spads.pl:6376-6395.
    expect(parseSkill('[14.61]', undefined)).toMatchObject({
      value: 14.61,
      origin: 'plugin',
    })
    expect(parseSkill('[#14.61#]', undefined)).toMatchObject({
      value: 14.61,
      origin: 'pluginDegraded',
    })
    expect(parseSkill('#15#', undefined)).toMatchObject({
      value: 15,
      origin: 'degraded',
    })
    expect(parseSkill('(3)', undefined)).toMatchObject({
      value: 3,
      origin: 'rank',
    })
    expect(parseSkill('~15', undefined)).toMatchObject({
      value: 15,
      origin: 'rounded',
    })
    expect(parseSkill('?15?', undefined)).toMatchObject({
      value: 15,
      origin: 'unknown',
    })
    expect(parseSkill('15.2', undefined)).toMatchObject({
      value: 15.2,
      origin: 'exact',
    })
  })

  test('a plugin blob is not mistaken for the degraded form', () => {
    // `[#n#]` also matches `[...]`, so ordering inside SIGILS matters.
    expect(parseSkill('[#7#]', undefined)?.origin).toBe('pluginDegraded')
  })

  test('a value it cannot read is no value at all', () => {
    expect(parseSkill(undefined, undefined)).toBeNull()
    expect(parseSkill('', undefined)).toBeNull()
    expect(parseSkill('[]', undefined)).toBeNull()
    expect(parseSkill('nonsense', undefined)).toBeNull()
  })

  test('an unreadable sigma leaves the skill standing', () => {
    expect(parseSkill('[14.61]', 'nope')).toMatchObject({
      value: 14.61,
      sigma: null,
    })
  })
})

describe('display', () => {
  test('past 6.65 the number is not worth stating', () => {
    // Chobby's threshold; 6.81 is the case that prompted this.
    expect(skillText({ value: 12, origin: 'plugin', sigma: 6.81 })).toBe('??')
    expect(skillText({ value: 12, origin: 'plugin', sigma: 6.65 })).toBe('12')
    expect(isUnrated({ value: 12, origin: 'plugin', sigma: null })).toBe(false)
  })

  test('a negative rating shows as zero, as Chobby does', () => {
    expect(skillText({ value: -3, origin: 'plugin', sigma: 1 })).toBe('0')
    expect(skillText({ value: 14.61, origin: 'plugin', sigma: 1 })).toBe('15')
  })

  test('brightness steps at 1.5, 2 and 3', () => {
    const at = (sigma: number | null) =>
      skillTier({ value: 20, origin: 'plugin', sigma })
    expect(at(1.49)).toBe(0)
    expect(at(1.5)).toBe(1)
    expect(at(2)).toBe(2)
    expect(at(3)).toBe(3)
    expect(at(9)).toBe(3)
    expect(at(null)).toBe(0)
  })
})

describe('readSkills', () => {
  const tags = {
    'game/players/drdandy/skill': '[23.4]',
    'game/players/drdandy/skilluncertainty': '0.9',
    'game/players/beanperson/skill': '[12.0]',
    'game/players/beanperson/skilluncertainty': '6.81',
    'game/players/nosigma/skill': '(4)',
    'game/hosttype': 'SPADS',
    'game/modoptions/tweakdefs1': 'bG9jYWw=',
  }

  test('picks up only the skill tags', () => {
    const skills = readSkills(tags)
    expect(Object.keys(skills).sort()).toEqual([
      'beanperson',
      'drdandy',
      'nosigma',
    ])
    expect(skills.drdandy).toMatchObject({ value: 23.4, sigma: 0.9 })
    expect(skills.nosigma).toMatchObject({ origin: 'rank', sigma: null })
  })

  test('an unrated player still carries a number for the team total', () => {
    const skills = readSkills(tags)
    expect(skillText(skills.beanperson!)).toBe('??')
    expect(teamSkill([skills.drdandy!, skills.beanperson!, null])).toBeCloseTo(
      35.4,
    )
  })

  test('no room means no skills', () => {
    expect(readSkills(undefined)).toEqual({})
  })
})
