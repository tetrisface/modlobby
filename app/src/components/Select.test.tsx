import { render } from '@solidjs/testing-library'
import { describe, expect, test } from 'vitest'
import { Select } from './Select'

/** Every component and view, as text. */
const SOURCES = import.meta.glob(['../**/*.tsx', '!../**/*.test.tsx'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

describe('Select', () => {
  test('is the one dropdown; a bare <select> anywhere else is refused', () => {
    const offenders = Object.entries(SOURCES)
      .filter(([path]) => !/(^|\/)Select\.tsx$/.test(path))
      .filter(([, source]) => /<select[\s>]/.test(source))
      .map(([path]) => path)
    expect(offenders).toEqual([])
  })

  test('draws the shared class, and keeps whatever else it was given', () => {
    const { container } = render(() => (
      <Select class='v-edit' value='b' aria-label='pick'>
        <option value='a'>A</option>
        <option value='b'>B</option>
      </Select>
    ))
    const select = container.querySelector('select') as HTMLSelectElement
    expect(select.className).toBe('select v-edit')
    expect(select.value).toBe('b')
    expect(select.getAttribute('aria-label')).toBe('pick')
  })
})
