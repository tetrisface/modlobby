import { invoke } from '../ipc/invoke'
import type { LanHostView } from '../ipc/bindings/LanHostView'
import type { LanRoomView } from '../ipc/bindings/LanRoomView'

/** What the host form fills in. */
export type HostForm = {
	title: string
	password: string | null
	approve: boolean
	maxPlayers: number
	engineVersion: string
	game: string
	map: string
}

/** The LAN's own commands; `ipc/client` is for servers, and this is not one. */
export const lanApi = {
	host: (form: HostForm) => invoke<LanHostView>('lan_host', form),
	start: () => invoke<void>('lan_start'),
	stop: () => invoke<void>('lan_stop'),
	status: () => invoke<LanHostView | null>('lan_status'),
	joinAddress: (address: string, password: string | null) =>
		invoke<void>('lan_join_address', { address, password }),
	rooms: () => invoke<LanRoomView[]>('lan_rooms'),
}
