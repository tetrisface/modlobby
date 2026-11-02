import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, expect, test } from 'vitest'
import { OptionName } from './OptionName'
import { Query } from './SettingRow'

afterEach(cleanup)

test('hovering the name tips its Lua key apart from its description', () => {
	const { container } = render(() => (
		<OptionName
			option={{
				key: 'deathmode',
				name: 'Game End Mode',
				desc: 'What it takes to eliminate a team',
				type: 'list',
			}}
		/>
	))
	const name = container.querySelector('.k') as HTMLElement
	expect(document.querySelector('.key-tip')).toBeNull()

	fireEvent.pointerEnter(name)
	const tip = document.querySelector('.key-tip')
	expect(tip?.textContent).toBe('deathmode: What it takes to eliminate a team')
	expect(tip?.querySelector('code')?.textContent).toBe('deathmode:')

	fireEvent.pointerLeave(name)
	expect(document.querySelector('.key-tip')).toBeNull()
})

test('the tip is drawn outside the list, which no scroller or container can clip', () => {
	const { container } = render(() => (
		<OptionName option={{ key: 'x', name: 'X', desc: 'Y', type: 'number' }} />
	))
	fireEvent.pointerEnter(container.querySelector('.k') as HTMLElement)
	expect(container.querySelector('.key-tip')).toBeNull()
	expect(document.querySelector('.key-tip')).not.toBeNull()
})

test('the tip of a number gives its range and its default', () => {
	const { container } = render(() => (
		<OptionName
			option={{
				key: 'maxunits',
				name: 'Max Units',
				type: 'number',
				def: 2000,
				min: 500,
				max: 32000,
				step: 1,
			}}
		/>
	))
	fireEvent.pointerEnter(container.querySelector('.k') as HTMLElement)
	expect(document.querySelector('.key-tip-facts')?.textContent).toBe(
		'min 500 · max 32000 · step 1 · default 2000',
	)
})

test('the tip of a list spells out its choices', () => {
	const { container } = render(() => (
		<OptionName
			option={{
				key: 'deathmode',
				name: 'Game End Mode',
				type: 'list',
				def: 'com',
				items: [
					{ key: 'com', name: 'Commanders' },
					{ key: 'killall', name: 'Everything' },
				],
			}}
		/>
	))
	fireEvent.pointerEnter(container.querySelector('.k') as HTMLElement)
	expect(document.querySelector('.key-tip-facts')?.textContent).toBe(
		'Commanders · Everything · default Commanders',
	)
})

test('a search marks what it found in the name', () => {
	const { container } = render(() => (
		<Query.Provider value={() => 'metal'}>
			<OptionName
				option={{ key: 'startmetal', name: 'Starting Metal', type: 'number' }}
			/>
		</Query.Provider>
	))
	expect(container.querySelector('.k')?.textContent).toBe('Starting Metal')
	expect(container.querySelector('mark')?.textContent).toBe('Metal')
})
