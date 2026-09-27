import { fireEvent } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { reorderGesture } from './drag'

/** A list of `count` rows with the gesture on every row. */
function list(count: number, onTap: () => void) {
	const parent = document.createElement('div')
	for (let i = 0; i < count; i++) {
		const row = document.createElement('div')
		row.textContent = `row ${i}`
		const press = reorderGesture({
			from: () => i,
			onOver: () => {},
			onDrop: () => {},
			onTap,
		})
		row.addEventListener('pointerdown', press as EventListener)
		parent.append(row)
	}
	document.body.append(parent)
	return parent
}

afterEach(() => {
	document.body.innerHTML = ''
})

describe('the reorder gesture', () => {
	test('a press let go where it began is a tap', () => {
		const tap = vi.fn()
		const rows = list(3, tap)
		fireEvent.pointerDown(rows.children[1]!, {
			button: 0,
			clientX: 40,
			clientY: 10,
		})
		fireEvent.pointerUp(window, { clientX: 40, clientY: 10 })
		expect(tap).toHaveBeenCalledTimes(1)
	})

	test('a press on a control inside a row is left to the control', () => {
		const tap = vi.fn()
		const rows = list(2, tap)
		const inner = document.createElement('button')
		inner.textContent = 'remove'
		rows.children[0]!.append(inner)
		fireEvent.pointerDown(inner, { button: 0, clientX: 5, clientY: 5 })
		fireEvent.pointerUp(window, { clientX: 5, clientY: 5 })
		expect(tap).not.toHaveBeenCalled()
	})
})
