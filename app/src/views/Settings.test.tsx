import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { fireEvent, render } from '@solidjs/testing-library'
import { beforeAll, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { applySettings } from '../store/settings'
import { SettingsView } from './Settings'

// The view talks to Rust when it saves. Nothing here waits for that, but the
// call must not be a missing import.
vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async () => {
		throw new Error('not in this test')
	}),
}))

function loaded(): Settings {
	return {
		$schema: null,
		lan: { enabled: false },
		servers: [
			{
				host: 'server4.beyondallreason.info',
				name: 'BAR',
				ports: [8200, 8201],
				allowUnencrypted: false,
				website: null,
				rapid: null,
				maps: null,
				username: 'me',
				channels: ['main'],
			},
		],
		account: { rememberPassword: false, autoLogin: false },
		connection: { idleDisconnectMinutes: 60 },
		paths: { dataDir: null },
		play: {
			joinAs: 'remember',
			lastWasPlayer: true,
			autoLaunch: true,
			autoDownload: true,
			pveStats: true,
		},
		notifications: {
			privateMessage: 'desktop',
			mention: 'desktop',
			ring: 'desktop',
			friendOnline: 'off',
			vote: 'lobby',
			gameStarting: 'desktop',
			gameEnded: 'desktop',
			doNotDisturb: false,
		},
		battleList: {
			showPassworded: true,
			showLocked: true,
			showRunning: true,
			showEmpty: false,
			friendsOnly: false,
			mode: 'all',
			sort: 'relevance',
			sortDescending: false,
		},
		chat: {
			filterHostChatter: true,
			maxLines: 3000,
			muted: ['main'],
		},
		overlay: {
			enabled: true,
			hotkey: 'Alt+Shift+L',
			returnFocusToGame: true,
			inGameEscape: true,
		},
		tweaks: { styluaConfig: null, defaultSlot: 'tweakdefs1' },
		logging: { filter: 'info' },
		updates: { automatic: true, download: true },
		ui: { scale: {} },
	}
}

/** The three buttons of one notification row, by the row's label. */
function choiceFor(container: HTMLElement, label: string) {
	const row = [...container.querySelectorAll('.choice-row')].find((element) =>
		element.textContent?.startsWith(label),
	)
	if (!row) throw new Error(`no row for ${label}`)
	const buttons = [...row.querySelectorAll('button')]
	return {
		click: (name: string) => {
			const button = buttons.find((b) => b.textContent?.trim() === name)
			if (!button) throw new Error(`no ${name} button`)
			fireEvent.click(button)
		},
		lit: () =>
			buttons
				.filter((button) => button.classList.contains('on'))
				.map((button) => button.textContent?.trim()),
	}
}

function openAt(address: string) {
	const history = createMemoryHistory()
	history.set({ value: address })
	return render(() => (
		<MemoryRouter history={history}>
			<Route path='/' component={SettingsView} />
		</MemoryRouter>
	))
}

/**
 * The view scrolled to its notifications.
 *
 * Opened at the address rather than through the index, because that address
 * is the feature: the corner notices link straight to this section, and the
 * section is a search parameter so that a link can name it.
 */
const openNotifications = () => openAt('/?section=notifications')

/** The rows a search has left showing, by their first line of text. */
function shown(container: HTMLElement): string[] {
	return [...container.querySelectorAll('.set-row')]
		.filter((row) => !row.hasAttribute('hidden'))
		.map((row) => row.textContent?.trim().split('\n')[0] ?? '')
}

function search(container: HTMLElement, query: string) {
	const box = container.querySelector<HTMLInputElement>(
		'input[aria-label="Search settings"]',
	)
	if (!box) throw new Error('no search box')
	fireEvent.input(box, { target: { value: query } })
}

/** The ids of the elements asked to scroll into view, in order. */
const scrolledTo: string[] = []

beforeAll(() => {
	// happy-dom lays nothing out, so there is nothing to scroll; what was asked
	// for is what can be checked.
	Element.prototype.scrollIntoView = function (this: Element) {
		scrolledTo.push(this.id)
	}
})

describe('finding a setting', () => {
	beforeEach(() => {
		applySettings(loaded())
		scrolledTo.length = 0
	})

	test('a word from a description finds the row it describes', () => {
		const { container } = openAt('/')
		search(container, 'metered')
		expect(shown(container)).toHaveLength(1)
		expect(shown(container)[0]).toContain(
			'Download what a room needs automatically',
		)
	})

	test('the words of a label find it, in any order', () => {
		const { container } = openAt('/')
		search(container, 'chatter bot')
		expect(shown(container)).toHaveLength(1)
		expect(shown(container)[0]).toContain('Filter bot chatter')
	})

	test("a section's title finds every row in it", () => {
		const { container } = openAt('/')
		const overlay = container.querySelector('#settings-overlay')
		search(container, 'overlay')
		const rows = overlay?.querySelectorAll('.set-row') ?? []
		expect(rows.length).toBeGreaterThan(1)
		for (const row of rows) expect(row.hasAttribute('hidden')).toBe(false)
	})

	test('clearing the search brings every row back', () => {
		const { container } = openAt('/')
		const every = container.querySelectorAll('.set-row').length
		search(container, 'metered')
		search(container, '')
		expect(shown(container)).toHaveLength(every)
	})

	test('what a notification means is on the page, not in a tooltip', () => {
		const { container } = openAt('/')
		expect(container.textContent).toContain(
			'which is how a host says the game is waiting on you',
		)
		expect(container.querySelector('.choice-row[title]')).toBeNull()
	})

	test('a link naming a section scrolls to it', () => {
		openNotifications()
		expect(scrolledTo).toEqual(['settings-notifications'])
	})

	test('the overview jumps to the section it names, and marks it', () => {
		const { container } = openAt('/')
		const overview = container.querySelector('.settings-overview')
		const chat = [...(overview?.querySelectorAll('button') ?? [])].find(
			(button) => button.textContent === 'Chat',
		)
		if (!chat) throw new Error('no Chat entry')
		fireEvent.click(chat)
		expect(scrolledTo).toEqual(['settings-chat'])
		expect(chat.classList.contains('on')).toBe(true)
		expect(chat.getAttribute('aria-current')).toBe('location')
	})

	test('the overview marks the section under the pointer, then the scroll again', () => {
		const { container } = openAt('/')
		const marked = () =>
			container.querySelector('.settings-overview .on')?.textContent
		expect(marked()).toBe('Account')

		const chatRow = container.querySelector('#settings-chat .set-row')!
		fireEvent.mouseOver(chatRow)
		expect(marked()).toBe('Chat')

		// Crossing the space between two sections is not leaving them.
		fireEvent.mouseOver(container.querySelector('.settings-body')!)
		expect(marked()).toBe('Chat')

		fireEvent.mouseLeave(container.querySelector('.settings-body')!)
		expect(marked()).toBe('Account')
	})

	test('while searching, the page itself is the overview', () => {
		const { container } = openAt('/')
		search(container, 'metered')
		expect(container.querySelector('.settings-overview')).toBeNull()
	})

	test('every section is headed by its name, with its rows beneath', () => {
		const { container } = openAt('/')
		const chat = container.querySelector('#settings-chat')
		expect(chat?.querySelector(':scope > h2')?.textContent).toBe('Chat')
		expect(
			chat?.querySelectorAll(':scope > .set-rows > .set-row').length,
		).toBeGreaterThan(0)
		expect(container.querySelector('fieldset')).toBeNull()
	})
})

describe('choosing where a notification goes', () => {
	beforeEach(() => applySettings(loaded()))

	test('do not disturb greys the rows it silences, and they stay changeable', () => {
		const { container } = openNotifications()
		const section = container.querySelector('#settings-notifications')!
		const silenced = () => section.querySelectorAll('.set-row.silenced').length
		expect(silenced()).toBe(0)

		const dnd = [...section.querySelectorAll('label')].find((label) =>
			label.textContent?.includes('Do not disturb'),
		)!
		fireEvent.click(dnd.querySelector('input')!)
		// The seven kinds and the line saying what their places mean; not the
		// switch itself.
		expect(silenced()).toBe(8)
		expect(dnd.closest('.set-row')?.classList.contains('silenced')).toBe(false)
		expect(section.querySelector('.silenced button:disabled')).toBeNull()
	})

	test('exactly one of the three is ever chosen', () => {
		const { container } = openNotifications()
		const mention = choiceFor(container, 'Someone says my name')

		expect(mention.lit()).toEqual(['Desktop'])

		mention.click('In lobby')
		expect(mention.lit()).toEqual(['In lobby'])

		mention.click('Desktop')
		expect(mention.lit()).toEqual(['Desktop'])

		mention.click('Off')
		expect(mention.lit()).toEqual(['Off'])
	})

	test('a row is chosen on its own, leaving its neighbours alone', () => {
		const { container } = openNotifications()
		const mention = choiceFor(container, 'Someone says my name')
		const ring = choiceFor(container, 'Someone rings me')

		mention.click('In lobby')
		expect(ring.lit()).toEqual(['Desktop'])
	})
})
