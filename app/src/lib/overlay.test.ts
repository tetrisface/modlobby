import { describe, expect, test } from 'vitest'
import { clickLeavesOverlay, escapeLeavesOverlay } from './overlay'

function key(overrides: Partial<Parameters<typeof escapeLeavesOverlay>[0]>) {
  return escapeLeavesOverlay({
    key: 'Escape',
    defaultPrevented: false,
    target: null,
    ...overrides,
  })
}

describe('escape while sitting over a game', () => {
  test('hands the game back', () => {
    expect(key({})).toBe(true)
  })

  test('is not any other key', () => {
    expect(key({ key: 'Enter' })).toBe(false)
    expect(key({ key: 'e' })).toBe(false)
  })

  test('leaves it to a dialog that already claimed it', () => {
    // Ask and the battle password sheet both close on Escape; the overlay
    // taking it as well would shut two things with one press.
    expect(key({ defaultPrevented: true })).toBe(false)
  })

  test('does not steal it out of a chat composer', () => {
    const input = document.createElement('input')
    expect(key({ target: input })).toBe(false)

    const area = document.createElement('textarea')
    expect(key({ target: area })).toBe(false)
  })

  test('still works from an ordinary part of the page', () => {
    const div = document.createElement('div')
    expect(key({ target: div })).toBe(true)
  })
})

describe('clicking while sitting over a game', () => {
  test('a click on the scrim hands the game back', () => {
    const outside = document.createElement('div')
    expect(clickLeavesOverlay(outside)).toBe(true)
  })

  test('a click inside the lobby does not', () => {
    const shell = document.createElement('div')
    shell.className = 'shell'
    const button = document.createElement('button')
    shell.append(button)
    expect(clickLeavesOverlay(button)).toBe(false)
    expect(clickLeavesOverlay(shell)).toBe(false)
  })

  test('a stray event with no element behind it changes nothing', () => {
    expect(clickLeavesOverlay(null)).toBe(false)
  })
})
