import { Room } from '../Room'
import { RoomProvider } from './model'
import { onlineRoom } from './online'

/**
 * The room route, on the server's side of the seam.
 *
 * The one place `onlineRoom()` is wired up, the way `store/tweakspaceInstance`
 * is the one place the tweak workspace meets Tauri.
 */
export function OnlineRoom() {
  return (
    <RoomProvider value={onlineRoom()}>
      <Room />
    </RoomProvider>
  )
}
