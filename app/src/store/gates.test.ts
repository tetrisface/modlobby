import { invoke } from '@tauri-apps/api/core'
import { expect, test, vi } from 'vitest'
import { battleKey } from '../lib/battles'
import { gateRefusals, joinRoom } from './gates'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const answer = vi.mocked(invoke)

test('a friends-only refusal is remembered until a join there gets through', async () => {
	const key = battleKey('server4', 7)

	answer.mockRejectedValueOnce({ code: 'refused', message: 'Battle locked' })
	await expect(joinRoom('server4', 7, null)).rejects.toBeDefined()
	expect(gateRefusals().has(key)).toBe(false)

	answer.mockRejectedValueOnce({
		code: 'refused',
		message: 'Friends only gatekeeper',
	})
	await expect(joinRoom('server4', 7, null)).rejects.toBeDefined()
	expect(gateRefusals().has(key)).toBe(true)

	answer.mockResolvedValueOnce(undefined)
	await joinRoom('server4', 7, null)
	expect(gateRefusals().has(key)).toBe(false)
})
