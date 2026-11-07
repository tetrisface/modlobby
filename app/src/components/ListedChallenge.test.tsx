import { cleanup, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test } from 'vitest'
import { ListedChallenge } from './ListedChallenge'

const SCORED_AT = '2026-11-07T18:00:00Z'
const threeMinutesOn = Date.parse(SCORED_AT) + 3 * 60_000

describe('ListedChallenge', () => {
	afterEach(cleanup)

	test('a room without a score shows nothing at all', () => {
		const { container } = render(() => (
			<ListedChallenge score={undefined} now={Date.now()} />
		))
		expect(container.textContent).toBe('')
		expect(container.querySelector('[title]')).toBeNull()
	})

	test('a filling room shows now–full, the full team in full colour', () => {
		const { container } = render(() => (
			<ListedChallenge
				score={{
					challenge: 12.4,
					fullChallenge: 15.1,
					players: 3,
					scoredAt: SCORED_AT,
				}}
				now={threeMinutesOn}
			/>
		))
		const cell = container.querySelector('[title]') as HTMLElement
		expect(cell.textContent).toBe('12–15')
		expect(cell.querySelector('.challenge-now')?.textContent).toBe('12')
		expect(cell.title).toContain('With every seat taken: 15.1')
		expect(cell.title).toContain('Last scored 3m ago')
	})

	test('a room with one figure shows it alone, unmuted', () => {
		const { container } = render(() => (
			<ListedChallenge
				score={{
					challenge: 24.85,
					fullChallenge: null,
					players: 8,
					scoredAt: SCORED_AT,
				}}
				now={threeMinutesOn}
			/>
		))
		const cell = container.querySelector('[title]') as HTMLElement
		expect(cell.textContent).toBe('25')
		expect(cell.querySelector('.challenge-now')).toBeNull()
		expect(cell.title).toContain(
			'With the 8 players seated when it was scored: 24.9',
		)
	})
})
