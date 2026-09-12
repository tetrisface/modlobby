import { createSignal } from 'solid-js'
import type { VersionView } from '../ipc/bindings/VersionView'

/** What this build is, read once at startup. */
export const [build, setBuild] = createSignal<VersionView | null>(null)

/**
 * Whether the engine here may be run against somebody's hosted game.
 *
 * False on macOS, where the only engine that exists is a third-party Apple
 * Silicon build its author asks not be used on the community servers. Rooms,
 * chat and the battle list cost those servers nothing and stay; a seat, a
 * ready flag and a launch are what go, because each of them ends in the engine
 * being started on somebody's game.
 *
 * This only shapes what is drawn. What actually holds the line is
 * `recoil::refuse_target`, at the one place the engine is ever spawned.
 *
 * Optimistic until the answer arrives: the call is one local IPC round trip
 * away, and every platform but one says yes.
 */
export const playsOnline = (): boolean => build()?.playsOnline ?? true

/**
 * Why no engine can be fetched onto this machine, when none can.
 *
 * `null` everywhere Beyond All Reason publishes a build, and `undefined` until
 * the shell has said which this is. Where it is a sentence, BAR's index has no
 * entry to ask for: the engine that runs here is a third-party bundle somebody
 * installed by hand, so every offer to fetch one, choose between them or try
 * again ends in a 404 the app then has to apologise for. The room says where
 * an engine comes from instead of making the offer.
 *
 * The words come from Rust rather than from here because the same fact is what
 * `download_engine` refuses with: two spellings of one refusal is how the room
 * and the runtime come to disagree about what this machine can do.
 *
 * The third state is the point, and it is why this does not read `?? null` the
 * way `playsOnline` above reads `?? true`. Both callers are one local round
 * trip from the answer and they want opposite things from the gap: what is
 * *drawn* would rather guess than flicker, so it takes not-knowing as no
 * refusal; what *asks for a download* would rather wait, so it gates on
 * `build()` itself. An offer a frame late is nothing. A request a frame early
 * is a red notice on the first room the machine opens.
 */
export const noPublishedEngine = (): string | null | undefined =>
  build()?.noPublishedEngine

/**
 * Whose engine this machine would fetch, when it is not Beyond All Reason's.
 *
 * `null` everywhere BAR publishes a build; on Apple Silicon the sentence that
 * names the port, says it is unaffiliated, and says the community servers are
 * not part of what it can do. Drawn beside the download rather than in front
 * of it — see `GetEngine`.
 *
 * `?? null` rather than gating on `build()` the way `noPublishedEngine` does,
 * because the two want opposite things from not-knowing: this only decides
 * what a notice *says*, and a notice one frame late is nothing, while a
 * download started one frame early is a request nobody can act on.
 */
export const thirdPartyEngine = (): string | null =>
  build()?.thirdPartyEngine ?? null
