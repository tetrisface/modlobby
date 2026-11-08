import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { asker } from './asking'

function deferred() {
	let resolve!: () => void
	const promise = new Promise<void>((yes) => {
		resolve = yes
	})
	return { promise, resolve }
}

async function settle() {
	for (let turn = 0; turn < 6; turn++) await Promise.resolve()
}

describe('asker', () => {
	beforeEach(() => {
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
	})
	afterEach(() => {
		vi.useRealTimers()
	})

	test('the first ask waits its stagger, then goes once', () => {
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 0, floor: 2000, stagger: () => 400 })
		asks.ask()
		asks.ask()
		vi.advanceTimersByTime(399)
		expect(run).not.toHaveBeenCalled()
		vi.advanceTimersByTime(1)
		expect(run).toHaveBeenCalledTimes(1)
	})

	test('asks while one is out become one follow-up, after it', async () => {
		const slow = deferred()
		const run = vi
			.fn()
			.mockReturnValueOnce(slow.promise)
			.mockResolvedValue(undefined)
		const asks = asker(run, { quiet: 0, floor: 0, stagger: () => 0 })
		asks.ask()
		vi.advanceTimersByTime(0)
		expect(run).toHaveBeenCalledTimes(1)

		asks.ask()
		asks.ask()
		asks.ask()
		vi.advanceTimersByTime(10_000)
		expect(run).toHaveBeenCalledTimes(1)

		slow.resolve()
		await settle()
		vi.advanceTimersByTime(0)
		expect(run).toHaveBeenCalledTimes(2)
	})

	test('two asks start no closer than the floor', async () => {
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 0, floor: 2000, stagger: () => 0 })
		asks.ask()
		vi.advanceTimersByTime(0)
		await settle()
		expect(run).toHaveBeenCalledTimes(1)

		vi.advanceTimersByTime(500)
		asks.ask()
		vi.advanceTimersByTime(1499)
		expect(run).toHaveBeenCalledTimes(1)
		vi.advanceTimersByTime(1)
		expect(run).toHaveBeenCalledTimes(2)
	})

	test('the stagger is read when the ask is scheduled, not when made', () => {
		let delay = 100
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 0, floor: 0, stagger: () => delay })
		delay = 700
		asks.ask()
		vi.advanceTimersByTime(699)
		expect(run).not.toHaveBeenCalled()
		vi.advanceTimersByTime(1)
		expect(run).toHaveBeenCalledTimes(1)
	})

	test('nothing goes after stop, not even a scheduled or remembered ask', async () => {
		const slow = deferred()
		const run = vi
			.fn()
			.mockReturnValueOnce(slow.promise)
			.mockResolvedValue(undefined)
		const asks = asker(run, { quiet: 0, floor: 0, stagger: () => 0 })
		asks.ask()
		vi.advanceTimersByTime(0)
		asks.ask()
		asks.stop()
		slow.resolve()
		await settle()
		vi.advanceTimersByTime(10_000)
		expect(run).toHaveBeenCalledTimes(1)

		asks.ask()
		vi.advanceTimersByTime(10_000)
		expect(run).toHaveBeenCalledTimes(1)
	})

	test('a run that rejects does not stop the next ask', async () => {
		const run = vi
			.fn()
			.mockRejectedValueOnce(new Error('no'))
			.mockResolvedValue(undefined)
		const asks = asker(run, { quiet: 0, floor: 0, stagger: () => 0 })
		asks.ask()
		vi.advanceTimersByTime(0)
		await settle()
		asks.ask()
		vi.advanceTimersByTime(0)
		expect(run).toHaveBeenCalledTimes(2)
	})

	test('a change after a quiet spell waits only its stagger', async () => {
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 3000, floor: 0, stagger: () => 400 })
		asks.ask()
		vi.advanceTimersByTime(400)
		await settle()
		expect(run).toHaveBeenCalledTimes(1)

		// Still for longer than the quiet: that wait is already done.
		vi.advanceTimersByTime(10_000)
		asks.ask()
		vi.advanceTimersByTime(399)
		expect(run).toHaveBeenCalledTimes(1)
		vi.advanceTimersByTime(1)
		expect(run).toHaveBeenCalledTimes(2)
	})

	test('a change inside a run of changes waits for the run to go quiet', async () => {
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 3000, floor: 0, stagger: () => 0 })
		asks.ask()
		vi.advanceTimersByTime(0)
		await settle()
		expect(run).toHaveBeenCalledTimes(1)

		vi.advanceTimersByTime(1000)
		asks.ask()
		vi.advanceTimersByTime(1000)
		asks.ask()
		// Quiet is counted from the last change, not the first.
		vi.advanceTimersByTime(2999)
		expect(run).toHaveBeenCalledTimes(1)
		// The settle timer hands over to the stagger's zero-length one, which
		// fake timers push a millisecond out when it is set inside a timer.
		vi.advanceTimersByTime(2)
		expect(run).toHaveBeenCalledTimes(2)
	})

	test('a change while an ask waits to go is covered by that ask', () => {
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 3000, floor: 0, stagger: () => 400 })
		asks.ask()
		vi.advanceTimersByTime(100)
		// It reads things as they are when it goes, this change included.
		asks.ask()
		vi.advanceTimersByTime(10_000)
		expect(run).toHaveBeenCalledTimes(1)
	})

	test('a change the last answer covers neither asks nor uses up the quiet', () => {
		let needed = false
		const run = vi.fn(async () => {})
		const asks = asker(
			run,
			{ quiet: 3000, floor: 0, stagger: () => 0 },
			() => needed,
		)
		asks.ask()
		vi.advanceTimersByTime(10_000)
		expect(run).not.toHaveBeenCalled()

		// A moment after the covered one, and still news after a quiet spell.
		vi.advanceTimersByTime(1)
		needed = true
		asks.ask()
		vi.advanceTimersByTime(0)
		expect(run).toHaveBeenCalledTimes(1)
	})

	test('an ask a later change made moot is dropped and costs no floor', () => {
		let needed = true
		const run = vi.fn(async () => {})
		const asks = asker(
			run,
			{ quiet: 0, floor: 60_000, stagger: () => 400 },
			() => needed,
		)
		asks.ask()
		needed = false
		vi.advanceTimersByTime(400)
		expect(run).not.toHaveBeenCalled()

		needed = true
		asks.ask()
		vi.advanceTimersByTime(400)
		expect(run).toHaveBeenCalledTimes(1)
	})

	test('after forget, the next change is news and the old run is dropped', async () => {
		const run = vi.fn(async () => {})
		const asks = asker(run, { quiet: 3000, floor: 0, stagger: () => 0 })
		asks.ask()
		vi.advanceTimersByTime(0)
		await settle()
		vi.advanceTimersByTime(1000)
		asks.ask()

		// Something new, a room say: what changed before is not its history.
		asks.forget()
		asks.ask()
		vi.advanceTimersByTime(0)
		expect(run).toHaveBeenCalledTimes(2)
		await settle()
		vi.advanceTimersByTime(10_000)
		expect(run).toHaveBeenCalledTimes(2)
	})
})
