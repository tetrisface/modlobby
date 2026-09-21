import { createRoot } from 'solid-js'
import { describe, expect, test, vi } from 'vitest'
import { grabKeys } from './keys'

function press(init: KeyboardEventInit) {
	const event = new KeyboardEvent('keydown', {
		bubbles: true,
		cancelable: true,
		...init,
	})
	document.body.dispatchEvent(event)
	return event
}

function editor() {
	const run = vi.fn(async () => {})
	return { run, focus: vi.fn(), getAction: vi.fn(() => ({ run })) }
}

describe('grabKeys', () => {
	test('Ctrl+F finds and Ctrl+P opens the palette in the editor, from anywhere, until it is gone', () => {
		const open = editor()
		const dispose = createRoot((dispose) => {
			grabKeys(() => open)
			return dispose
		})

		expect(press({ key: 'f', ctrlKey: true }).defaultPrevented).toBe(true)
		expect(open.focus).toHaveBeenCalled()
		expect(open.getAction).toHaveBeenLastCalledWith('actions.find')
		expect(press({ key: 'p', ctrlKey: true }).defaultPrevented).toBe(true)
		expect(open.getAction).toHaveBeenLastCalledWith(
			'editor.action.quickCommand',
		)
		expect(open.run).toHaveBeenCalledTimes(2)

		// Not ours: a plain f, Ctrl+Shift+F, and any other Ctrl key.
		expect(press({ key: 'f' }).defaultPrevented).toBe(false)
		expect(
			press({ key: 'F', ctrlKey: true, shiftKey: true }).defaultPrevented,
		).toBe(false)
		expect(press({ key: 's', ctrlKey: true }).defaultPrevented).toBe(false)

		dispose()
		expect(press({ key: 'f', ctrlKey: true }).defaultPrevented).toBe(false)
		expect(open.run).toHaveBeenCalledTimes(2)
	})

	test('does nothing while there is no editor yet', () => {
		const dispose = createRoot((dispose) => {
			grabKeys(() => undefined)
			return dispose
		})
		expect(press({ key: 'f', ctrlKey: true }).defaultPrevented).toBe(false)
		dispose()
	})
})
