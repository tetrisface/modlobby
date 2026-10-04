import { fireEvent, render } from '@solidjs/testing-library'
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
