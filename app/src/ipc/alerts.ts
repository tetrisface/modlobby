import { UserAttentionType, getCurrentWindow } from '@tauri-apps/api/window'
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification'
import type { Alert } from './bindings/Alert'
import type { AlertKind } from './bindings/AlertKind'
import { api } from './client'
import { pushNotice } from '../store/chat'
import { settings } from '../store/settings'

/**
 * Saying that something happened, as loudly as the settings ask for.
 *
 * Three choices that do three different things, and never each other's. `off`
 * says nothing. `lobby` puts a line in this window's corner. `desktop` raises
 * a notification from the operating system, and only while the window is in
 * the background — a desktop toast for something already on screen is noise,
 * which is the line Chobby draws too (`api_notification_handler.lua`).
 *
 * `desktop` deliberately does not fall back to the lobby's corner. It did, and
 * that made the choices overlap: picking `desktop` also got you what `lobby`
 * does. Nothing is lost by the silence — every one of these events leaves its
 * own mark in the window as well, an unread badge, a marked line, a vote bar.
 *
 * A desktop alert also flashes the taskbar entry, which is the half of this
 * that always works: a toast can be refused, missed, or swallowed by a focus
 * assistant, while a flashing button stays flashing until you look. Chobby
 * does the same thing, though it has to reach into another process to do it
 * (`dist_cfg/exts/os_notifications.js` calls `FlashWindowEx` with caption and
 * tray flags, which is what `Critical` asks for here).
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

/**
 * What to do about an alert, given where it is meant to go and whether anyone
 * is looking at the window.
 *
 * Separated out because it is the whole decision, and the rest of this file is
 * plumbing that cannot be tested without a desktop.
 */
export function plan(
  where: Alert,
  focused: boolean,
): 'nothing' | 'lobby' | 'desktop' {
  if (where === 'off') return 'nothing'
  if (where === 'lobby') return 'lobby'
  return focused ? 'nothing' : 'desktop'
}

/** Said once, not once per alert, when the desktop will not play along. */
let refused = false

function cannot(): void {
  if (refused) return
  refused = true
  pushNotice(
    'warning',
    'your desktop is not letting modlobby raise notifications, so anything set to Desktop will stay quiet',
  )
}

export async function raise(kind: AlertKind, body: string): Promise<void> {
  const what = plan(wanted(kind), document.hasFocus())
  if (what === 'nothing') return
  if (what === 'lobby') return pushNotice('info', body)

  void flash(kind)
  if (!(await allowed())) return cannot()
  try {
    sendNotification({ title: TITLES[kind], body })
  } catch {
    cannot()
  }
}

/** Alerts about the game itself, which is the window you would rather see. */
const ABOUT_THE_GAME: ReadonlySet<AlertKind> = new Set<AlertKind>([
  'gameStarting',
  'gameEnded',
])

/**
 * How long to keep looking for the engine's window before deciding it is not
 * there. A game that is starting has not opened one yet; one that has ended
 * may still be closing it.
 */
export const ENGINE_WINDOW_WAIT_MS = 3000
const ENGINE_WINDOW_POLL_MS = 250

/**
 * Which taskbar entry to flash for an alert, once the engine has or has not
 * been found.
 *
 * A game starting is news about the engine's window and nothing else: the
 * lobby is what you are leaving, and flashing it would point the wrong way,
 * so when there is no engine window there is nothing to flash. A game ending
 * is news about the engine too, but by then it may be gone -- and the lobby
 * is where you are headed, so it stands in. Everything else is about the lobby.
 */
export function flashTarget(
  kind: AlertKind,
  engineFound: boolean,
): 'engine' | 'lobby' | 'nothing' {
  if (!ABOUT_THE_GAME.has(kind)) return 'lobby'
  if (engineFound) return 'engine'
  return kind === 'gameStarting' ? 'nothing' : 'lobby'
}

/**
 * Keeps trying `attempt` until it answers true or the time is up.
 * `now` and `sleep` are parameters so a test can run it on a fake clock.
 */
export async function keepTrying(
  attempt: () => Promise<boolean>,
  total: number,
  every: number,
  now: () => number = Date.now,
  sleep: (ms: number) => Promise<void> = (ms) =>
    new Promise((resolve) => setTimeout(resolve, ms)),
): Promise<boolean> {
  const deadline = now() + total
  for (;;) {
    if (await attempt()) return true
    if (now() >= deadline) return false
    await sleep(every)
  }
}

/**
 * Flashes a taskbar entry until it is looked at.
 *
 * The engine is a process we spawned rather than a window we own, so that
 * half goes through Rust and `FlashWindowEx`, which answers whether it found
 * a window at all. It is given a few seconds to appear, since a starting game
 * is still loading; see [`flashTarget`] for what happens when it does not.
 */
async function flash(kind: AlertKind): Promise<void> {
  try {
    const engineFound =
      ABOUT_THE_GAME.has(kind) &&
      (await keepTrying(
        () => api.flashEngine(),
        ENGINE_WINDOW_WAIT_MS,
        ENGINE_WINDOW_POLL_MS,
      ))
    if (flashTarget(kind, engineFound) !== 'lobby') return
    await getCurrentWindow().requestUserAttention(UserAttentionType.Critical)
  } catch {
    // A platform with no taskbar to flash is not an error; the notification is
    // the point and it has already gone out.
  }
}
