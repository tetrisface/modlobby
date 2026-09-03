import { describe, expect, test } from 'vitest'
import { activityReporter } from './activity'

describe('activityReporter', () => {
  test('the first event reports at once', () => {
    let reports = 0
    const touch = activityReporter(
      () => (reports += 1),
      1000,
      () => 0,
    )
    touch()
    expect(reports).toBe(1)
  })

  test('events inside the window are folded into the last report', () => {
    let clock = 0
    let reports = 0
    const touch = activityReporter(
      () => (reports += 1),
      1000,
      () => clock,
    )
    touch()
    clock = 999
    touch()
    touch()
    expect(reports).toBe(1)
    clock = 1000
    touch()
    expect(reports).toBe(2)
  })

  test('the window counts from the last report, not the last event', () => {
    let clock = 0
    let reports = 0
    const touch = activityReporter(
      () => (reports += 1),
      1000,
      () => clock,
    )
    touch()
    clock = 900
    touch()
    clock = 1100
    touch()
    expect(reports).toBe(2)
  })
})
