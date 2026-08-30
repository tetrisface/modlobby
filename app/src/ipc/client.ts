import { invoke, type Channel } from '@tauri-apps/api/core'
import type { DiffView } from './bindings/DiffView'
import type { Kind } from './bindings/Kind'
import type { Prepared } from './bindings/Prepared'
import type { ReplayView } from './bindings/ReplayView'
import type { SkirmishOptions } from './bindings/SkirmishOptions'
import type { Settings } from './bindings/Settings'
import type { Slot } from './bindings/Slot'
import type { TweakView } from './bindings/TweakView'
import type { UiMessage } from './bindings/UiMessage'

/** The shape every failed command rejects with (`ApiError` in commands.rs). */
export type ApiError = { code: string; message: string }

export type VoteChoice = 'y' | 'n' | 'b'

export const api = {
  subscribe: (channel: Channel<UiMessage>) =>
    invoke<void>('subscribe', { channel }),
  login: (
    username: string,
    password: string | null,
    remember: boolean,
    autoLogin: boolean,
  ) => invoke<void>('login', { username, password, remember, autoLogin }),
  logout: () => invoke<void>('logout'),
  loginWait: () => invoke<number>('login_wait'),
  joinBattle: (id: number, password: string | null) =>
    invoke<void>('join_battle', { id, password }),
  leaveBattle: () => invoke<void>('leave_battle'),
  rememberedBattle: () => invoke<number | null>('remembered_battle'),
  forgetBattle: () => invoke<void>('forget_battle'),
  launch: () => invoke<void>('launch'),
  sayBattle: (text: string) => invoke<void>('say_battle', { text }),
  vote: (choice: VoteChoice) => invoke<void>('vote', { choice }),
  setOption: (key: string, value: string) =>
    invoke<void>('set_option', { key, value }),
  joinChannel: (room: string, key: string | null) =>
    invoke<void>('join_channel', { room, key }),
  leaveChannel: (room: string) => invoke<void>('leave_channel', { room }),
  sayChannel: (room: string, text: string) =>
    invoke<void>('say_channel', { room, text }),
  sayPrivate: (user: string, text: string) =>
    invoke<void>('say_private', { user, text }),
  listChannels: () => invoke<void>('list_channels'),
  downloadMissing: () => invoke<void>('download_missing'),
  stopDownload: () => invoke<void>('stop_download'),
  ring: (user: string) => invoke<void>('ring', { user }),
  setAway: (away: boolean) => invoke<void>('set_away', { away }),
  rememberChannels: (channels: string[]) =>
    invoke<Settings>('remember_channels', { channels }),
  skirmishOptions: () => invoke<SkirmishOptions>('skirmish_options'),
  startSkirmish: (
    game: string,
    map: string,
    engine: string,
    opponents: string[],
  ) => invoke<void>('start_skirmish', { game, map, engine, opponents }),
  listReplays: () => invoke<ReplayView[]>('list_replays'),
  playReplay: (path: string) => invoke<void>('play_replay', { path }),
  refreshFriends: () => invoke<void>('refresh_friends'),
  friendAction: (
    action: 'request' | 'accept' | 'decline' | 'remove' | 'ignore' | 'unignore',
    user: string,
  ) => invoke<void>('friend_action', { action, user }),
  getSettings: () => invoke<Settings>('get_settings'),
  updateSettings: (settings: Settings) =>
    invoke<Settings>('update_settings', { settings }),
  hasPassword: (username: string) =>
    invoke<boolean>('has_password', { username }),
  clearPassword: (username: string) =>
    invoke<void>('clear_password', { username }),
  openSettingsFile: () => invoke<void>('open_settings_file'),
  openDataDir: () => invoke<void>('open_data_dir'),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  openLogDir: () => invoke<void>('open_log_dir'),

  takeSeat: (team: number, allyTeam: number) =>
    invoke<void>('take_seat', { team, allyTeam }),
  releaseSeat: () => invoke<void>('release_seat'),
  setReady: (ready: boolean) => invoke<void>('set_ready', { ready }),
  setSide: (side: number) => invoke<void>('set_side', { side }),
  requestPrivateHost: (region: string) =>
    invoke<string>('request_private_host', { region }),
  hostPublic: (region: string) => invoke<number>('host_public', { region }),

  tweakDecode: (blob: string, kind: Kind) =>
    invoke<TweakView>('tweak_decode', { blob, kind }),
  tweakFormat: (lua: string, kind: Kind) =>
    invoke<string>('tweak_format', { lua, kind }),
  tweakPrepare: (lua: string, slot: Slot, direct: boolean) =>
    invoke<Prepared>('tweak_prepare', { lua, slot, direct }),
  tweakSend: (lua: string, slot: Slot, direct: boolean) =>
    invoke<Prepared>('tweak_send', { lua, slot, direct }),
  tweakClear: (slot: Slot) => invoke<void>('tweak_clear', { slot }),
  tweakDiff: (kind: Kind, current: string, proposed: string) =>
    invoke<DiffView>('tweak_diff', { kind, current, proposed }),
  listDrafts: () => invoke<string[]>('list_drafts'),
  readDraft: (name: string) => invoke<string>('read_draft', { name }),
  saveDraft: (name: string, lua: string) =>
    invoke<void>('save_draft', { name, lua }),
  deleteDraft: (name: string) => invoke<void>('delete_draft', { name }),
}

/** The twenty slots, in the order the game applies them. */
export const SLOTS: { slot: Slot; key: string; kind: Kind }[] = [
  ...Array.from({ length: 10 }, (_, index) => ({
    slot: { kind: 'defs', index } as Slot,
    key: index === 0 ? 'tweakdefs' : `tweakdefs${index}`,
    kind: 'defs' as Kind,
  })),
  ...Array.from({ length: 10 }, (_, index) => ({
    slot: { kind: 'units', index } as Slot,
    key: index === 0 ? 'tweakunits' : `tweakunits${index}`,
    kind: 'units' as Kind,
  })),
]

export function describeError(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as ApiError).message)
  }
  return String(error)
}
