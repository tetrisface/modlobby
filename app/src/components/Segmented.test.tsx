import { fireEvent, render } from '@solidjs/testing-library'
import { createSignal } from 'solid-js'
import { describe, expect, test, vi } from 'vitest'
import { Segmented } from './Segmented'

const OPTIONS = [
	{ value: 'changed', label: 'Changed' },
	{ value: 'all', label: 'All' },
] as const

describe('Segmented', () => {
	test('marks the one that is on and hands back the one pressed', () => {
		const onChange = vi.fn()
		const { getByText } = render(() => (
			<Segmented
				label='Show'
				value='changed'
				options={OPTIONS}
				onChange={onChange}
			/>
		))
		expect(getByText('Changed').getAttribute('aria-pressed')).toBe('true')
		expect(getByText('All').getAttribute('aria-pressed')).toBe('false')
		fireEvent.click(getByText('All'))
		expect(onChange).toHaveBeenCalledWith('all')
	})

	test('a label that changes keeps its button, and a segment its own classes', () => {
		const [count, setCount] = createSignal(1)
		const { getByText } = render(() => (
			<Segmented
				label='Sent'
				value='asIs'
				options={[
					{
						value: 'asIs',
						label: `as is ${count()}`,
						classList: { over: count() > 1 },
					},
				]}
				onChange={() => {}}
			/>
		))
		const before = getByText('as is 1')
		expect(before.className).toBe('on')
		setCount(2)
		expect(getByText('as is 2')).toBe(before)
		expect(before.className.split(' ').sort()).toEqual(['on', 'over'])
	})

	test('greyed, it says why and takes no press', () => {
		const onChange = vi.fn()
		const { getByRole, getByText } = render(() => (
			<Segmented
				label='Show'
				value='changed'
				options={OPTIONS}
				onChange={onChange}
				disabled
				title='A group shows every setting in it'
			/>
		))
		expect(getByRole('group').title).toBe('A group shows every setting in it')
		fireEvent.click(getByText('All'))
		expect(onChange).not.toHaveBeenCalled()
	})
})
