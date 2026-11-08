import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { Room } from '../Room'
import { type Calls, battle, fakeRoom, myBattle, recordingIo } from './fixture'
import { RoomProvider } from './model'
import { Welcome } from './Welcome'

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(),
	convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

beforeEach(() => {
	vi.mocked(invoke).mockImplementation(async (command: string) => {
		switch (command) {
			case 'game_modoptions':
			case 'game_ais':
			case 'game_unit_names':
				return []
			case 'engine_def_tags':
				return { weapon: [] }
			default:
				return null
		}
	})
})

afterEach(() => {
	cleanup()
	vi.clearAllMocks()
	vi.restoreAllMocks()
})

const NUTTYB =
	'This lobby was created using https://nuttyb-community.github.io/nuttyb/configurator. Links for widgets and useful resources can be found there on the About page.'

/** The text box as tall as `clipped` lines when it holds `full`. */
function layOut(full: number, clipped: number): void {
	const box = (el: Element) => el.classList.contains('room-welcome-text')
	vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(
		function (this: HTMLElement) {
			return box(this) ? full : 0
		},
	)
	vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(
		function (this: HTMLElement) {
			return box(this) ? clipped : 0
		},
	)
}

test('a paragraph a line, with its links made clickable', () => {
	const { container } = render(() => <Welcome lines={[NUTTYB, 'Be nice.']} />)
	const paragraphs = container.querySelectorAll('.room-welcome-text p')
	expect([...paragraphs].map((p) => p.textContent)).toEqual([
		NUTTYB,
		'Be nice.',
	])
	const link = container.querySelector<HTMLAnchorElement>('a.chat-link')
	expect(link?.getAttribute('href')).toBe(
		'https://nuttyb-community.github.io/nuttyb/configurator',
	)
})

test('a welcome that fits has nothing to open', () => {
	layOut(54, 54)
	const { container } = render(() => <Welcome lines={['Be nice.']} />)
	expect(container.querySelector('.room-welcome-more')).toBeNull()
})

test('a longer welcome opens in place, and closes again', () => {
	layOut(200, 54)
	const { container } = render(() => <Welcome lines={[NUTTYB, NUTTYB]} />)
	const more = container.querySelector<HTMLButtonElement>('.room-welcome-more')
	expect(more?.textContent).toBe('more')
	fireEvent.click(more!)
	expect(more?.textContent).toBe('less')
	expect(more?.getAttribute('aria-expanded')).toBe('true')
	expect(
		container.querySelector('.room-welcome-text')?.classList.contains('open'),
	).toBe(true)
	fireEvent.click(more!)
	expect(more?.textContent).toBe('more')
})

function mountRoom(welcome: string[]) {
	const calls: Calls = []
	const room = fakeRoom({
		battle: () => battle(),
		my: () => myBattle({ welcome }),
		io: recordingIo(calls),
	})
	return render(() => (
		<MemoryRouter
			root={(props) => (
				<RoomProvider value={room}>{props.children}</RoomProvider>
			)}
		>
			<Route path='/' component={Room} />
		</MemoryRouter>
	))
}

test("the room's welcome sits on its card", async () => {
	const { container } = mountRoom([NUTTYB])
	const card = await waitFor(() => {
		const found = container.querySelector('.room-card')
		if (!found) throw new Error('no card')
		return found
	})
	expect(card.classList.contains('welcomed')).toBe(true)
	expect(card.querySelector('.room-welcome')?.textContent).toContain(
		'About page',
	)
})

test('a room with nothing to say draws no note', async () => {
	const { container } = mountRoom([])
	const card = await waitFor(() => {
		const found = container.querySelector('.room-card')
		if (!found) throw new Error('no card')
		return found
	})
	expect(card.classList.contains('welcomed')).toBe(false)
	expect(card.querySelector('.room-welcome')).toBeNull()
})
