import { cleanup, render } from '@solidjs/testing-library'
import { describe, expect, test } from 'vitest'
import { clickLeavesOverlay } from '../lib/overlay'
import { CardGrips } from './CardGrips'

function pointer(type: string, clientX: number) {
	return new MouseEvent(type, { clientX, bubbles: true, button: 0 })
}

/** Node's own `localStorage` hides jsdom's, so the test brings one. */
function storage() {
	const held = new Map<string, string>()
	Object.defineProperty(window, 'localStorage', {
		configurable: true,
		value: {
			getItem: (key: string) => held.get(key) ?? null,
			setItem: (key: string, value: string) => void held.set(key, value),
		},
	})
}

/** The card on a 2560×1440 monitor, under a title bar Tauri drags. */
function card() {
	Object.defineProperty(window, 'innerWidth', {
		value: 2560,
		configurable: true,
	})
	Object.defineProperty(window, 'innerHeight', {
		value: 1440,
		configurable: true,
	})
	return render(() => (
		<div class='shell'>
			<nav class='nav' data-tauri-drag-region='true' />
			<CardGrips />
		</div>
	))
}

/** The right edge dragged 500px in. */
function narrowed(container: HTMLElement) {
	const edge = container.querySelector('.card-grip.e') as HTMLElement
	edge.dispatchEvent(pointer('pointerdown', 2520))
	window.dispatchEvent(pointer('pointermove', 2020))
	window.dispatchEvent(pointer('pointerup', 2020))
}

const page = document.documentElement.style

describe('the card over a game', () => {
	test('drags in by its edge, stays the lobby, and is kept for next time', () => {
		storage()
		const { container } = card()
		expect(page.getPropertyValue('--card-right')).toBe('40px')

		// Pressed on the card's edge and released over the game, it is still a
		// press in the lobby: otherwise every resize would hand the game back.
		expect(clickLeavesOverlay(container.querySelector('.card-grip.e'))).toBe(
			false,
		)
		narrowed(container)
		expect(page.getPropertyValue('--card-right')).toBe('540px')
		expect(page.getPropertyValue('--card-left')).toBe('40px')

		cleanup()
		card()
		expect(page.getPropertyValue('--card-right'), 'the next game').toBe('540px')
		cleanup()
	})

	test('a double click on the title bar puts it back, past Tauri', () => {
		storage()
		// What Tauri's drag script does with a press on a drag region, from the
		// document, before anything delegated there could see it.
		const tauri = (event: MouseEvent) => {
			if ((event.target as Element).hasAttribute('data-tauri-drag-region'))
				event.stopImmediatePropagation()
		}
		document.addEventListener('mousedown', tauri)
		const { container } = card()
		narrowed(container)
		const nav = container.querySelector('nav') as HTMLElement
		const press = (detail: number) =>
			nav.dispatchEvent(
				new MouseEvent('mousedown', { bubbles: true, button: 0, detail }),
			)

		press(1)
		expect(page.getPropertyValue('--card-right'), 'a drag').toBe('540px')
		press(2)
		expect(page.getPropertyValue('--card-right')).toBe('40px')

		cleanup()
		card()
		expect(page.getPropertyValue('--card-right'), 'the next game').toBe('40px')
		cleanup()
		document.removeEventListener('mousedown', tauri)
	})
})
