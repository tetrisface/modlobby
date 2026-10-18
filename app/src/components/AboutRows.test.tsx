import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { setBuild } from '../store/build'
import { AboutRows } from './AboutRows'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
	invoke: (...args: unknown[]) => invoke(...args),
}))

afterEach(() => {
	cleanup()
	invoke.mockReset()
	setBuild(null)
})

const REPO = 'https://github.com/tetrisface/modlobby'
const PAGE = 'https://example.test/support'

function open(support: string | null = null) {
	return render(() => <AboutRows repo={REPO} support={support} />)
}

/** The buttons on the page, by what they say. */
function buttons(container: HTMLElement): string[] {
	return [...container.querySelectorAll('button')].map(
		(button) => button.textContent?.trim() ?? '',
	)
}

function click(container: HTMLElement, label: string) {
	const button = [...container.querySelectorAll('button')].find((b) =>
		b.textContent?.includes(label),
	)
	if (!button) throw new Error(`no ${label} button`)
	fireEvent.click(button)
}

describe('about', () => {
	test('the build is written out as version plus commit', () => {
		setBuild({ version: '0.1.43', commit: 'a29589c', playsOnline: true })
		const { container } = open()
		expect(container.textContent).toContain('modlobby 0.1.43+a29589c')
	})

	test('the source button hands the repository to the browser', async () => {
		invoke.mockResolvedValue(undefined)
		const { container } = open()
		click(container, 'GitHub')
		await waitFor(() =>
			expect(invoke).toHaveBeenCalledWith('open_url', { url: REPO }),
		)
	})

	test('with no page to give to, there is no support button', () => {
		const { container } = open(null)
		expect(buttons(container).some((b) => b.includes('Support'))).toBe(false)
	})

	test('the support button hands the page to the browser', async () => {
		invoke.mockResolvedValue(undefined)
		const { container } = open(PAGE)
		click(container, 'Support modlobby')
		await waitFor(() =>
			expect(invoke).toHaveBeenCalledWith('open_url', { url: PAGE }),
		)
	})
})
