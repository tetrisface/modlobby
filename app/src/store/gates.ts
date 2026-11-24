import { createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { battleKey } from '../lib/battles'

/**
 * The rooms whose friends-only gate turned us away, by battle list key.
 *
 * Teiserver tells a room's gatekeeper only to the people already in it
 * (`s.battle.extra_data`, sent with the join), so from the list a refusal is
 * the one sign of it. It holds until a join there gets through: the room's
 * boss can lift the gate without the list ever hearing.
 */
const [gateRefusals, setGateRefusals] = createSignal<ReadonlySet<string>>(
	new Set(),
)
export { gateRefusals }

/** Teiserver's reason, word for word (`consul_server.ex`, `allow_join`). */
const FRIENDS_ONLY = 'Friends only gatekeeper'

/** Joins a room, remembering whether its friends-only gate said no. */
export async function joinRoom(
	server: string,
	id: number,
	password: string | null,
): Promise<void> {
	const key = battleKey(server, id)
	try {
		await api.joinBattle(server, id, password)
	} catch (error) {
		if (describeError(error) === FRIENDS_ONLY)
			setGateRefusals((held) => new Set(held).add(key))
		throw error
	}
	if (gateRefusals().has(key))
		setGateRefusals((held) => new Set([...held].filter((k) => k !== key)))
}
