import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification'
import type { Alert } from './bindings/Alert'
import type { AlertKind } from './bindings/AlertKind'
import { pushNotice } from '../store/chat'
import { settings } from '../store/settings'

/**
 * Saying that something happened, as loudly as the settings ask for.
 *
 * Each kind of event is set to one of three things. `off` says nothing.
 * `lobby` puts a line in the lobby's own corner. `desktop` raises a real
 * notification — but only while the window is in the background, falling back
 * to the corner when it is not, because a desktop toast for something already
 * on screen is noise. Chobby draws the same line
 * (`api_notification_handler.lua`).
 *
 * Permission is asked for once, lazily, so someone who sets everything to
 * `lobby` or `off` is never prompted for it.
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

/** Where this kind of event is meant to be said, if anywhere. */
function wanted(kind: AlertKind): Alert {
  const notifications = settings()?.notifications
  if (!notifications) return 'off'
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
    case 'gameEnded':
      return notifications.gameEnded
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
  gameEnded: 'modlobby — game finished',
  ring: 'modlobby — someone wants you',
}

export async function raise(kind: AlertKind, body: string): Promise<void> {
  const where = wanted(kind)
  if (where === 'off') return
  if (where === 'lobby' || document.hasFocus()) return pushNotice('info', body)

  if (!(await allowed())) return pushNotice('info', body)
  try {
    sendNotification({ title: TITLES[kind], body })
  } catch {
    // A desktop that refuses notifications is not a reason to lose the event.
    pushNotice('info', body)
  }
}
