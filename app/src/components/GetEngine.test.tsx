import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { GetEngine, forgetAskedEngines } from './GetEngine'

/**
 * The one download modlobby does itself.
 *
 * A fresh machine opens its room with no engine version at all, and that is
 * the version it asks for: Rust reads it as the newest there is.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}))

async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

/** The commands that reached Tauri, with what they carried. */
function sent(command: string) {
	return vi
		.mocked(invoke)
		.mock.calls.filter(([name]) => name === command)
		.map(([, args]) => args)
}

beforeEach(() => {
	vi.mocked(invoke).mockResolvedValue(null)
})

afterEach(() => {
	cleanup()
	// What `auto` has already asked for outlives a mount on purpose, so it has
	// to be forgotten between tests.
	forgetAskedEngines()
	vi.clearAllMocks()
})

describe('getting the first engine onto a machine', () => {
	test('a version it was given is fetched once, when it may be', async () => {
		render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
	})

	test('a room with no version asks for the newest engine, once', async () => {
		// The first run: nothing installed, so the room names nothing. That is
		// a question with an answer -- whatever BAR plays on today -- not a 404.
		const { getByText } = render(() => <GetEngine version='' auto />)
		await settle()
		expect(sent('download_engine')).toEqual([{ version: '' }])

		fireEvent.click(getByText('Download the engine'))
		await settle()
		expect(sent('download_engine')).toHaveLength(2)
	})

	test('a room that remounts does not ask the index again', async () => {
		// A tab away and back, a list rebuilding its rows: each of those used to
		// be another trip to BAR's index for a version it may well have no build
		// for, and each 404 arrived as a red notice about the network. (A page
		// load is a new page and asks once, which is the floor intended.)
		const first = render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		first.unmount()

		const second = render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
		second.unmount()

		// A version nobody has asked about yet is still a new question.
		render(() => <GetEngine version='2026.09.01' auto />)
		await settle()
		expect(sent('download_engine')).toEqual([
			{ version: '2026.07.04' },
			{ version: '2026.09.01' },
		])
	})

	test('an answer about the version is kept, one about the network is not', async () => {
		// The index saying there is no such build is settled: a remount gets
		// nothing new by asking. A connection that dropped is about the moment,
		// and the next mount after it may ask — or a laptop that was briefly
		// offline would go the whole session without its engine, silently.
		vi.mocked(invoke).mockRejectedValueOnce({
			code: 'network',
			message: 'fetching the engine: connection reset',
		})
		const dropped = render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		dropped.unmount()
		render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		expect(sent('download_engine')).toHaveLength(2)
		cleanup()
		vi.clearAllMocks()
		vi.mocked(invoke).mockResolvedValue(null)

		vi.mocked(invoke).mockRejectedValueOnce({
			code: 'notFound',
			message: "BAR's index has no engine_windows64 build of engine 2026.09.01",
		})
		const missing = render(() => <GetEngine version='2026.09.01' auto />)
		await settle()
		missing.unmount()
		render(() => <GetEngine version='2026.09.01' auto />)
		await settle()
		expect(sent('download_engine')).toHaveLength(1)
	})

	test('a page reload remembers what was asked', async () => {
		// A reload is a fresh module: the variable starts empty. The window is
		// the same, though, and so is its sessionStorage, which is what carries
		// the answer across.
		render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		expect(sent('download_engine')).toHaveLength(1)
		cleanup()

		vi.resetModules()
		const reloaded = await import('./GetEngine')
		render(() => <reloaded.GetEngine version='2026.07.04' auto />)
		await settle()
		expect(sent('download_engine')).toHaveLength(1)

		// And it is the memory that stopped it, not a dead effect.
		render(() => <reloaded.GetEngine version='2026.09.01' auto />)
		await settle()
		expect(sent('download_engine')).toHaveLength(2)
		reloaded.forgetAskedEngines()
	})

	test('a click asks again for a version already tried on its own', async () => {
		// The standing question is only about asking unprompted. Somebody who
		// clicks after a failure means it, and gets the whole answer back.
		const { getByText } = render(() => <GetEngine version='2026.07.04' auto />)
		await settle()
		expect(sent('download_engine')).toHaveLength(1)

		fireEvent.click(getByText('Download the engine'))
		await settle()
		expect(sent('download_engine')).toHaveLength(2)
	})

	test('the heading names the version when there is one', async () => {
		const named = render(() => <GetEngine version='2026.07.04' />)
		await settle()
		expect(named.getByText('Engine 2026.07.04 is not installed.')).toBeTruthy()
		named.unmount()

		// Rather than "Engine  is not installed.", with the gap where it went.
		const bare = render(() => <GetEngine version='' />)
		await settle()
		expect(bare.getByText('The engine is not installed.')).toBeTruthy()
	})
})
