import { describe, expect, test } from 'vitest'
import { shortcut } from './platform'

describe('shortcut', () => {
	test('is Ctrl and its keys everywhere but a Mac', () => {
		expect(shortcut('S', false)).toBe('Ctrl+S')
		expect(shortcut('Shift+F', false)).toBe('Ctrl+Shift+F')
	})

	test('is written the Apple way on a Mac', () => {
		expect(shortcut('S', true)).toBe('⌘S')
		expect(shortcut('Shift+F', true)).toBe('⇧⌘F')
		expect(shortcut('Enter', true)).toBe('⌘↩')
		expect(shortcut('Shift+Z', true)).toBe('⇧⌘Z')
	})
})
