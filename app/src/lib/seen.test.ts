import { describe, expect, it } from 'vitest'

import { readSeen, rememberSeen, seenText } from './seen'

const NOW = Date.parse('2026-09-21T12:00:00Z')
const HOUR = 3600

function memory(): Storage {
	const held = new Map<string, string>()
	return {
		getItem: (key) => held.get(key) ?? null,
		setItem: (key, value) => void held.set(key, value),
	} as Storage
}

describe('seen', () => {
	it('keeps each server apart and gives it back', () => {
		const storage = memory()
		rememberSeen(storage, 'a', { users: 3, rooms: 1, at: 10 })
		rememberSeen(storage, 'b', { users: 7, rooms: 2, at: 20 })
		expect(readSeen(storage, 'a')).toEqual({ users: 3, rooms: 1, at: 10 })
		expect(readSeen(storage, 'b')).toEqual({ users: 7, rooms: 2, at: 20 })
		expect(readSeen(storage, 'c')).toBeNull()
	})

	it('reads nothing from a store that is garbled or not there', () => {
		const storage = memory()
		storage.setItem('servers.seen', '{ not json')
		expect(readSeen(storage, 'a')).toBeNull()
		expect(readSeen(null, 'a')).toBeNull()
	})

	it('says the live count as it is and a remembered one with its age', () => {
		const earlier = { users: 312, rooms: 41, at: NOW / 1000 - 2 * HOUR }
		expect(seenText({ users: 312, rooms: 1 }, earlier, NOW)).toBe(
			'312 online, 1 room',
		)
		expect(seenText(null, earlier, NOW)).toBe('312 online, 41 rooms, 2h ago')
		expect(seenText(null, null, NOW)).toBeNull()
	})
})
