import { invoke, type Channel } from '@tauri-apps/api/core'
import type { Settings } from './bindings/Settings'
import type { UiMessage } from './bindings/UiMessage'

/** The shape every failed command rejects with (`ApiError` in commands.rs). */
export type ApiError = { code: string; message: string }

export type VoteChoice = 'y' | 'n' | 'b'

export const api = {
  subscribe: (channel: Channel<UiMessage>) =>
    invoke<void>('subscribe', { channel }),
  login: (username: string, password: string | null, remember: boolean) =>
    invoke<void>('login', { username, password, remember }),
  logout: () => invoke<void>('logout'),
  joinBattle: (id: number, password: string | null) =>
    invoke<void>('join_battle', { id, password }),
  leaveBattle: () => invoke<void>('leave_battle'),
  launch: () => invoke<void>('launch'),
  sayBattle: (text: string) => invoke<void>('say_battle', { text }),
  vote: (choice: VoteChoice) => invoke<void>('vote', { choice }),
  getSettings: () => invoke<Settings>('get_settings'),
  updateSettings: (settings: Settings) =>
    invoke<Settings>('update_settings', { settings }),
  hasPassword: (username: string) =>
    invoke<boolean>('has_password', { username }),
  clearPassword: (username: string) =>
    invoke<void>('clear_password', { username }),
  openSettingsFile: () => invoke<void>('open_settings_file'),
  openDataDir: () => invoke<void>('open_data_dir'),
}

export function describeError(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as ApiError).message)
  }
  return String(error)
}
