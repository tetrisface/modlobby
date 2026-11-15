import { describe, expect, test } from 'vitest'
import {
	INSETS,
	type Insets,
	dragInsets,
	fitInsets,
	readInsets,
	writeInsets,
} from './card'

const VIEW = { width: 2560, height: 1440 }
const SMALLEST = { width: 900, height: 600 }

describe('dragging an edge of the card', () => {
	test('moves the edge with the pointer, and a corner moves two', () => {
		// The right edge dragged left is a wider margin on the right.
		expect(dragInsets(INSETS, 'e', -100, 0, VIEW).right).toBe(140)
		// The top edge dragged down is a wider margin on top.
		expect(dragInsets(INSETS, 'n', 0, 100, VIEW).top).toBe(140)
		expect(dragInsets(INSETS, 'sw', 30, -20, VIEW)).toEqual({
			top: 40,
			right: 40,
			bottom: 60,
			left: 70,
		})
	})

	test('stops at the window, and where the card would be too small', () => {
		expect(dragInsets(INSETS, 'w', -500, 0, VIEW).left).toBe(0)
		// 2560 wide less the 40 on the left leaves 1620 before 900 is reached.
		expect(dragInsets(INSETS, 'e', -5000, 0, VIEW, SMALLEST).right).toBe(1620)
		expect(dragInsets(INSETS, 's', 0, -5000, VIEW, SMALLEST).bottom).toBe(800)
	})
})

describe('a saved card on another monitor', () => {
	test('is kept as it is where it fits', () => {
		const saved: Insets = { top: 100, right: 600, bottom: 100, left: 40 }
		expect(fitInsets(saved, VIEW, SMALLEST)).toEqual(saved)
	})

	test('shrinks its margins in proportion where it does not', () => {
		const saved: Insets = { top: 40, right: 1200, bottom: 40, left: 400 }
		// 1920 less 900 leaves 1020 to share, a quarter of it on the left.
		expect(fitInsets(saved, { width: 1920, height: 1080 }, SMALLEST)).toEqual({
			top: 40,
			right: 765,
			bottom: 40,
			left: 255,
		})
	})
})

describe('the card remembered', () => {
	function store() {
		const held = new Map<string, string>()
		return {
			getItem: (key: string) => held.get(key) ?? null,
			setItem: (key: string, value: string) => void held.set(key, value),
		}
	}

	test('comes back as it was left, and forgetting it leaves nothing', () => {
		const storage = store()
		const left: Insets = { top: 10, right: 20, bottom: 30, left: 40 }
		writeInsets(storage, 'card', left)
		expect(readInsets(storage, 'card')).toEqual(left)
		writeInsets(storage, 'card', null)
		expect(readInsets(storage, 'card')).toBeNull()
	})

	test('anything else written there is not trusted', () => {
		const storage = store()
		for (const junk of ['{', '[1,2,3]', '[1,2,3,-4]', '"40"', '[1,2,3,null]']) {
			storage.setItem('card', junk)
			expect(readInsets(storage, 'card'), junk).toBeNull()
		}
	})
})
