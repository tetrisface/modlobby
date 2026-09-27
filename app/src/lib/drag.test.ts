import { fireEvent } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { reorderGesture } from './drag'

/** A list of `count` rows, each `tag`, with the gesture on every row. */
function list(tag: 'div' | 'button', count: number, onTap: () => void) {
	const parent = document.createElement('div')
	for (let i = 0; i < count; i++) {
		const row = document.createElement(tag)
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
	test('a press let go where it began is a tap, on a row that is itself a button', () => {
		const tap = vi.fn()
		const chips = list('button', 3, tap)
		fireEvent.pointerDown(chips.children[1]!, {
			button: 0,
			clientX: 40,
			clientY: 10,
		})
		fireEvent.pointerUp(window, { clientX: 40, clientY: 10 })
		expect(tap).toHaveBeenCalledTimes(1)
	})

	test('a label before the rows does not shift where a chip lands', () => {
		const over = vi.fn()
		const drop = vi.fn()
		const parent = document.createElement('div')
		const label = document.createElement('span')
		label.textContent = 'Sort'
		parent.append(label)
		for (let i = 0; i < 3; i++) {
			const chip = document.createElement('button')
			chip.className = 'chip'
			chip.addEventListener(
				'pointerdown',
				reorderGesture({
					axis: 'x',
					rows: '.chip',
					from: () => i,
					onOver: over,
					onDrop: drop,
				}) as EventListener,
			)
			parent.append(chip)
		}
		document.body.append(parent)
		// jsdom lays nothing out: the label at 0-20, then a chip every 20px.
		;[...parent.children].forEach((child, at) => {
			child.getBoundingClientRect = () =>
				({ left: at * 20, width: 20, top: 0, height: 10 }) as DOMRect
		})
		fireEvent.pointerDown(parent.children[3]!, {
			button: 0,
			clientX: 70,
			clientY: 5,
		})
		fireEvent.pointerMove(window, { clientX: 25, clientY: 5 })
		expect(over).toHaveBeenLastCalledWith({ from: 2, to: 0 })
		fireEvent.pointerUp(window, { clientX: 25, clientY: 5 })
		expect(drop).toHaveBeenCalledWith(2, 0)
	})

	test('a press on a control inside a row is left to the control', () => {
		const tap = vi.fn()
		const rows = list('div', 2, tap)
		const inner = document.createElement('button')
		inner.textContent = 'remove'
		rows.children[0]!.append(inner)
		fireEvent.pointerDown(inner, { button: 0, clientX: 5, clientY: 5 })
		fireEvent.pointerUp(window, { clientX: 5, clientY: 5 })
		expect(tap).not.toHaveBeenCalled()
	})
})
