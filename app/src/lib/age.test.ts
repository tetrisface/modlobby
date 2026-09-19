import { describe, expect, it } from 'vitest'

import { age, exactly } from './age'

const NOW = Date.parse('2026-09-19T12:00:00Z')
const ago = (days: number) => new Date(NOW - days * 86_400_000).toISOString()

describe('age', () => {
  it('reads as the largest unit and the next', () => {
    expect(age(ago(400), NOW)).toBe('1y 1m ago')
    expect(age(ago(102), NOW)).toBe('3m 12d ago')
    expect(age(ago(5), NOW)).toBe('5d ago')
  })

  it('leaves out a next unit that is zero', () => {
    expect(age(ago(365), NOW)).toBe('1y ago')
    expect(age(ago(60), NOW)).toBe('2m ago')
  })

  it('calls anything under a day today, and a clock ahead of the data too', () => {
    expect(age(ago(0.5), NOW)).toBe('today')
    expect(age(ago(-1), NOW)).toBe('today')
  })

  it('says nothing about a date nobody published', () => {
    expect(age('', NOW)).toBe('')
    expect(exactly('')).toBe('')
  })
})
