/**
 * Seeding the lobby store in tests. Not imported by the app: the runtime is
 * what fills the store there.
 */
import { reconcile } from 'solid-js/store'
import {
	emptyLobby,
	emptyServer,
	lobby,
	setLobby,
	type ServerState,
} from './lobby'

/** The server a test's one session is on. */
export const TEST_SERVER = 'server4'

/** Sets part of a session, opening it first if there is none. */
export function seedSession(
	patch: Partial<ServerState>,
	server = TEST_SERVER,
): void {
	setLobby('servers', server, {
		...(lobby.servers[server] ?? emptyServer()),
		...patch,
	})
}

/** Replaces the whole store with one session, and nothing of the machine's. */
export function onlySession(session: ServerState, server = TEST_SERVER): void {
	setLobby(reconcile({ ...emptyLobby(), servers: { [server]: session } }))
}
