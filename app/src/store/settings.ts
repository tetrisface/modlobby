import { createSignal } from 'solid-js'
import type { Settings } from '../ipc/bindings/Settings'
import { api } from '../ipc/client'
import { bucket, clamp, derived, scaleFor, step } from '../lib/scale'
import { pushNotice, setChat } from './chat'

/** Mirror of the settings file; the runtime pushes changes as `settings` events. */
export const [settings, setSettingsSignal] = createSignal<Settings | null>(null)

/** How long a run of wheel notches settles before the file is written. */
const SAVE_AFTER = 600

/** What the interface is drawn at now, as a percentage. */
export const [uiScale, setUiScale] = createSignal(100)

let pending: ReturnType<typeof setTimeout> | undefined

/** The screen the window is on. Absent under a test runner. */
function display(): { width: number; height: number } {
  const screen = typeof window === 'undefined' ? undefined : window.screen
  return { width: screen?.width ?? 1920, height: screen?.height ?? 1080 }
}

/**
 * Size the interface.
 *
 * Every length in `styles.css` is `rem` against the root, and the root is
 * `calc(16px * var(--ui-scale))`, so this one property is the whole of it.
 */
function draw(percent: number): void {
  setUiScale(percent)
  document.documentElement.style.setProperty(
    '--ui-scale',
    String(percent / 100),
  )
}

export function applySettings(next: Settings): void {
  setSettingsSignal(next)
  setChat('maxLines', next.chat.maxLines)
  setChat('filterHostChatter', next.chat.filterHostChatter)
  const { width, height } = display()
  // `ui` is optional only in the moment a reloaded front end meets a runtime
  // built before this field existed, which is a development-only skew — but
  // the cost of it is a lobby that will not draw, so it is worth the `??`.
  draw(scaleFor(next.ui?.scale ?? {}, width, height))
}

/**
 * Draw at a new size and remember it for this screen.
 *
 * Drawn at once, so the gesture reads as direct; written back only once the
 * wheel stops, since a single flick sends a dozen of these and each write
 * edits the settings file in place.
 */
export function setScale(percent: number): void {
  const { width, height } = display()
  const next = clamp(percent, width, height)
  draw(next)
  const current = settings()
  if (current === null) return
  const written = {
    ...current,
    ui: {
      ...current.ui,
      scale: { ...current.ui.scale, [bucket(width, height)]: next },
    },
  }
  setSettingsSignal(written)
  clearTimeout(pending)
  pending = setTimeout(() => {
    api
      .updateSettings(written)
      .catch(() => pushNotice('warning', 'could not save the interface size'))
  }, SAVE_AFTER)
}

/** One notch of the wheel. */
export function nudgeScale(direction: 1 | -1): void {
  setScale(step(uiScale(), direction))
}

/** Back to the size this screen suits. */
export function resetScale(): void {
  const { width, height } = display()
  setScale(derived(width, height))
}
