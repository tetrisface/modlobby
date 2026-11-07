import { cleanup, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test } from 'vitest'
import { ListedChallenge } from './ListedChallenge'

const SCORED_AT = '2026-11-07T18:00:00Z'

describe('ListedChallenge', () => {
	afterEach(cleanup)

	test('a room without a score shows nothing at all', () => {
		const { container } = render(() => (
			<ListedChallenge score={undefined} now={Date.now()} />
		))
		expect(container.textContent).toBe('')
		expect(container.querySelector('[title]')).toBeNull()
	})

	test('a scored room shows the figure, with its scale and age on hover', () => {
		const { getByText } = render(() => (
			<ListedChallenge
				score={{ challenge: 24.85, scoredAt: SCORED_AT }}
				now={Date.parse(SCORED_AT) + 3 * 60_000}
			/>
		))
		const figure = getByText('24.9')
		expect(figure.title).toContain('Challenge 24.9 of 34')
		expect(figure.title).toContain('Last scored 3m ago')
	})
})
