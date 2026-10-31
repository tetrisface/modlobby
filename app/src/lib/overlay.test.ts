import { describe, expect, test } from 'vitest'
import {
	clickAway,
	clickLeavesOverlay,
	escapeLeavesOverlay,
	roomOnScreen,
} from './overlay'

function key(overrides: Partial<Parameters<typeof escapeLeavesOverlay>[0]>) {
	return escapeLeavesOverlay({
		key: 'Escape',
		defaultPrevented: false,
		repeat: false,
		target: null,
		...overrides,
	})
}

describe('escape while sitting over a game', () => {
	test('hands the game back', () => {
		expect(key({})).toBe(true)
	})

	test('is a fresh press, never the tail of the one that raised it', () => {
		// The in-game Escape raises the lobby and focuses it mid-press. Held
		// past the keyboard's repeat delay, the key's repeats land here, and
		// taking them would put the game straight back.
		expect(key({ repeat: true })).toBe(false)
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

describe('a click-away from press to release', () => {
	function gesture() {
		const scrim = document.createElement('div')
		const shell = document.createElement('div')
		shell.className = 'shell'
		let left = 0
		const away = clickAway(() => {
			left += 1
		})
		return { scrim, shell, away, left: () => left }
	}

	test('leaves on the release, with the button up, never on the press', () => {
		// A game handed focus mid-press loses its next clicks on the Lobby button.
		const { scrim, away, left } = gesture()
		away.press({ target: scrim })
		expect(left()).toBe(0)
		away.release({ target: scrim })
		expect(left()).toBe(1)
	})

	test('a text selection dragged out of the card stays', () => {
		const { scrim, shell, away, left } = gesture()
		away.press({ target: shell })
		away.release({ target: scrim })
		expect(left()).toBe(0)
	})

	test('a press that wanders back into the card stays', () => {
		const { scrim, shell, away, left } = gesture()
		away.press({ target: scrim })
		away.release({ target: shell })
		expect(left()).toBe(0)
	})

	test('a release with no press of its own stays', () => {
		const { scrim, away, left } = gesture()
		away.release({ target: scrim })
		expect(left()).toBe(0)
	})

	test('one gesture leaves once', () => {
		const { scrim, away, left } = gesture()
		away.press({ target: scrim })
		away.release({ target: scrim })
		away.release({ target: scrim })
		expect(left()).toBe(1)
	})
})

describe('the corner buttons while a room is on screen', () => {
	test('a room page carries its own, so the corner stays empty', () => {
		expect(roomOnScreen('/room')).toBe(true)
		expect(roomOnScreen('/room/tweaks')).toBe(true)
		expect(roomOnScreen('/skirmish')).toBe(true)
	})

	test('every other page gets them in the corner', () => {
		expect(roomOnScreen('/')).toBe(false)
		expect(roomOnScreen('/battles')).toBe(false)
		expect(roomOnScreen('/settings')).toBe(false)
		expect(roomOnScreen('/roomy')).toBe(false)
	})
})
