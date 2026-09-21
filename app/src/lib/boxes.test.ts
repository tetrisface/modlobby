import { describe, expect, it } from 'vitest'
import { boxSignature, centre, extent, isBoxKey, outline } from './boxes'

describe('recognising a start-box modoption', () => {
	it('takes both keys, bare or under their scripttag prefix', () => {
		expect(isBoxKey('mapmetadata_startbox_override')).toBe(true)
		expect(isBoxKey('game/modoptions/mapmetadata_startboxes_set')).toBe(true)
	})

	it('leaves other modoptions alone', () => {
		expect(isBoxKey('game/modoptions/tweakdefs1')).toBe(false)
		expect(isBoxKey('startbox')).toBe(false)
	})
})

describe('the signature that says the boxes moved', () => {
	it('changes when either key changes', () => {
		const before = boxSignature({
			'game/modoptions/mapmetadata_startbox_override': 'aaa',
		})
		const after = boxSignature({
			'game/modoptions/mapmetadata_startbox_override': 'bbb',
		})
		expect(after).not.toBe(before)
	})

	it('is the same for a room with nothing set and no room at all', () => {
		expect(boxSignature({})).toBe(boxSignature(undefined))
	})

	it('does not confuse one key being set with the other', () => {
		const override = boxSignature({
			'game/modoptions/mapmetadata_startbox_override': 'x',
		})
		const set = boxSignature({
			'game/modoptions/mapmetadata_startboxes_set': 'x',
		})
		expect(override).not.toBe(set)
	})
})

describe('drawing a box', () => {
	const square: [number, number][] = [
		[0, 0],
		[100, 0],
		[100, 100],
		[0, 100],
	]

	it('moves to the first corner, lines to the rest, and closes', () => {
		expect(outline(square)).toBe('M0 0 L100 0 L100 100 L0 100 Z')
	})

	it('puts the label in the middle', () => {
		expect(centre(square)).toEqual({ x: 50, y: 50 })
	})

	it('has an answer for a polygon with no corners rather than NaN', () => {
		expect(centre([])).toEqual({ x: 100, y: 100 })
	})
})

describe('a box made of islands', () => {
	// Three squares in a row joined by seams at y=100; the end ones are the
	// deepest, the middle one is where the box as a whole is.
	const islands: [number, number][] = [
		[20, 80],
		[60, 80],
		[60, 100],
		[90, 100],
		[90, 82],
		[126, 82],
		[126, 100],
		[150, 100],
		[150, 80],
		[190, 80],
		[190, 120],
		[150, 120],
		[150, 100],
		[126, 100],
		[126, 118],
		[90, 118],
		[90, 100],
		[60, 100],
		[60, 120],
		[20, 120],
	]

	it('labels the middle island rather than the deepest one', () => {
		const { x, y } = centre(islands)
		expect(Math.abs(x - 108)).toBeLessThan(1)
		expect(Math.abs(y - 100)).toBeLessThan(1)
	})
})

describe('a box that surrounds another', () => {
	// The whole map less a square hole: in along a seam at y=100, round the
	// hole the other way, and out along the same seam.
	const keyhole: [number, number][] = [
		[0, 0],
		[200, 0],
		[200, 200],
		[0, 200],
		[0, 100],
		[60, 100],
		[60, 140],
		[140, 140],
		[140, 60],
		[60, 60],
		[60, 100],
		[0, 100],
	]

	it('puts the label in the band, where the average of the corners would put it in the hole', () => {
		const { x, y } = centre(keyhole)
		// As deep in the band as it goes: 30 from both sides of it.
		const fromBorder = Math.min(x, y, 200 - x, 200 - y)
		const fromHole = Math.hypot(
			Math.max(60 - x, 0, x - 140),
			Math.max(60 - y, 0, y - 140),
		)
		expect(Math.min(fromBorder, fromHole)).toBeGreaterThan(29)
	})
})

describe('the rectangle a polygon fits in', () => {
	it('takes the far corners whatever order the points came in', () => {
		expect(
			extent([
				[60, 20],
				[20, 50],
				[40, 80],
			]),
		).toEqual({ left: 20, top: 20, right: 60, bottom: 80 })
	})

	it('gives a polygon with nothing in it a rectangle with nothing in it', () => {
		expect(extent([])).toEqual({ left: 0, top: 0, right: 0, bottom: 0 })
	})
})
