import { createSignal } from 'solid-js'
import type { Settings } from '../ipc/bindings/Settings'
import { setChat } from './chat'

/** Mirror of the settings file; the runtime pushes changes as `settings` events. */
export const [settings, setSettingsSignal] = createSignal<Settings | null>(null)

export function applySettings(next: Settings): void {
  setSettingsSignal(next)
  setChat('maxLines', next.chat.maxLines)
}
