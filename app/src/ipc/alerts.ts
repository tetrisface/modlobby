import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification'
import type { AlertKind } from './bindings/AlertKind'
import { settings } from '../store/settings'

/**
 * Raising an OS notification for something worth interrupting for.
 *
 * Only while the window is in the background: a toast for something already on
 * screen is noise, which is the same line Chobby draws
 * (`api_notification_handler.lua`). Permission is asked for once, lazily, so a
 * user who never enables notifications is never prompted.
 */

let permission: Promise<boolean> | null = null

function allowed(): Promise<boolean> {
  permission ??= (async () => {
    try {
      if (await isPermissionGranted()) return true
      return (await requestPermission()) === 'granted'
    } catch {
      return false
    }
  })()
  return permission
}

/** Which setting governs each kind. */
function wanted(kind: AlertKind): boolean {
  const notifications = settings()?.notifications
  if (!notifications?.enabled) return false
  switch (kind) {
    case 'privateMessage':
      return notifications.privateMessage
    case 'mention':
      return notifications.mention
    case 'friendOnline':
      return notifications.friendOnline
    case 'vote':
      return notifications.vote
    case 'gameStarting':
      return notifications.gameStarting
    case 'ring':
      return notifications.ring
  }
}

const TITLES: Record<AlertKind, string> = {
  privateMessage: 'modlobby — message',
  mention: 'modlobby — you were named',
  friendOnline: 'modlobby — a friend is online',
  vote: 'modlobby — vote',
  gameStarting: 'modlobby — game starting',
  ring: 'modlobby — someone wants you',
}

export async function raise(kind: AlertKind, body: string): Promise<void> {
  if (document.hasFocus()) return
  if (!wanted(kind)) return
  if (!(await allowed())) return
  try {
    sendNotification({ title: TITLES[kind], body })
  } catch {
    // A desktop that refuses notifications is not an error worth reporting;
    // the line is in the app either way.
  }
}
