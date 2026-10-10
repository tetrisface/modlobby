import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, expect, test } from 'vitest'
import { OptionName } from './OptionName'

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
	expect(container.querySelector('.key-tip')).toBeNull()

	fireEvent.pointerEnter(name)
	const tip = container.querySelector('.key-tip')
	expect(tip?.textContent).toBe('deathmode: What it takes to eliminate a team')
	expect(tip?.querySelector('code')?.textContent).toBe('deathmode:')

	fireEvent.pointerLeave(name)
	expect(container.querySelector('.key-tip')).toBeNull()
})
