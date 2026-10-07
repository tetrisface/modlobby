import { cleanup, render } from '@solidjs/testing-library'
import { afterEach, expect, test } from 'vitest'
import { IconSprite } from './icons'

afterEach(cleanup)

/** The chevrons a path draws, by their arm-end y; each starts at the left arm. */
const chevrons = (path: Element | null): number[] =>
	[...(path?.getAttribute('d')?.matchAll(/M[\d.]+ ([\d.]+)/g) ?? [])].map(
		(hit) => Number(hit[1]),
	)

test('ranks 5-8 gild the four-stack from the bottom', () => {
	const { container } = render(() => <IconSprite />)
	const drawn = (level: number) => {
		const symbol = container.querySelector(`#chev${level}`)!
		return {
			silver: chevrons(symbol.querySelector('[stroke="var(--silver)"]')),
			gold: chevrons(symbol.querySelector('[fill="var(--gold)"]')),
		}
	}
	expect(drawn(4).silver).toHaveLength(4)
	expect(drawn(4).gold).toHaveLength(0)
	expect(drawn(8).silver).toHaveLength(0)
	expect(drawn(8).gold).toHaveLength(4)

	const five = drawn(5)
	expect(five.silver).toHaveLength(3)
	expect(five.gold).toHaveLength(1)
	// Larger y is lower on screen: the gilt chevron is the bottom one.
	expect(five.gold[0]).toBeGreaterThan(Math.max(...five.silver))
})
