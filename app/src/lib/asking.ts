/**
 * One question at a time, not too often, and not all at once.
 *
 * Written for the PvE score panel, whose service is a Lambda with a
 * concurrency of one and a twenty-second cold start: asking twice while an
 * answer is out gets two cold starts, every client in a room asking at the
 * same instant gets one answer and a row of refusals, and a lobby that asks
 * on every settings change goes through somebody else's budget for nothing.
 * Nothing in here knows what is being asked.
 */

export type Pace = {
	/**
	 * How long nothing must have changed for a change to be asked about at
	 * once. The stillness before a change counts: a change after a quiet
	 * spell waits only its stagger, while one inside a run of changes waits
	 * for the run to have been still this long, so a host working through a
	 * preset is asked about as it starts and once more when it is done.
	 */
	quiet: number
	/** The least time between the starts of two asks. */
	floor: number
	/** How long to wait before an ask, read when the ask is scheduled. */
	stagger: () => number
}

export type Asker = {
	/** Something changed: asks about it, or arranges to, as the pace allows. */
	ask(): void
	/** As for something new: the changes so far no longer count against the quiet. */
	forget(): void
	/** Nothing further happens after this. */
	stop(): void
}

/**
 * `run` does the asking and owns its own failures; a rejection is swallowed
 * here so a timer never surfaces it.
 *
 * `needed` says whether there is anything to ask: whether the last answer
 * falls short of things as they are now. A change that finds nothing on its
 * way is let in only when it is needed, so a change the last answer already
 * covers neither asks nor uses up the quiet. An ask about to go is dropped
 * when a later change made it moot, and does not count against the floor.
 *
 * Once something is on its way, every change joins it. An ask waiting to go
 * reads whatever is current when it goes, so it covers the changes before
 * it; changes while one is out are remembered, once, as a follow-up.
 */
export function asker(
	run: () => Promise<void>,
	pace: Pace,
	needed: () => boolean = () => true,
): Asker {
	let inFlight = false
	let again = false
	let stopped = false
	let lastStart = Number.NEGATIVE_INFINITY
	let lastChange = Number.NEGATIVE_INFINITY
	/** An ask waiting out its stagger or the floor. */
	let timer: ReturnType<typeof setTimeout> | undefined
	/** A run of changes waiting to go quiet. */
	let settling: ReturnType<typeof setTimeout> | undefined

	const onItsWay = () =>
		inFlight || timer !== undefined || settling !== undefined

	async function go() {
		timer = undefined
		if (!needed()) return
		inFlight = true
		lastStart = Date.now()
		try {
			await run()
		} catch {
			// run reports its own failures
		} finally {
			inFlight = false
		}
		if (stopped || !again) return
		again = false
		schedule()
	}

	/** Puts one ask on its way: never more than one out, never sooner than the floor. */
	function schedule() {
		if (inFlight) {
			again = true
			return
		}
		if (timer !== undefined) return
		const wait = Math.max(pace.stagger(), lastStart + pace.floor - Date.now())
		timer = setTimeout(() => void go(), wait)
	}

	function settled() {
		settling = undefined
		schedule()
	}

	function ask() {
		if (stopped) return
		if (!onItsWay() && !needed()) return
		const now = Date.now()
		const still = now - lastChange >= pace.quiet
		lastChange = now
		clearTimeout(settling)
		settling = undefined
		if (timer !== undefined) return
		if (still) {
			schedule()
			return
		}
		settling = setTimeout(settled, pace.quiet)
	}

	return {
		ask,
		forget() {
			lastChange = Number.NEGATIVE_INFINITY
			clearTimeout(settling)
			settling = undefined
		},
		stop() {
			stopped = true
			clearTimeout(timer)
			clearTimeout(settling)
			timer = undefined
			settling = undefined
		},
	}
}
