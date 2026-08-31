import { invoke, type Channel } from '@tauri-apps/api/core'
import type { DiffView } from './bindings/DiffView'
import type { Kind } from './bindings/Kind'
import type { Prepared } from './bindings/Prepared'
import type { ReplayView } from './bindings/ReplayView'
import type { SkirmishOptions } from './bindings/SkirmishOptions'
import type { BoxesView } from './bindings/BoxesView'
import type { Book } from './bindings/Book'
import type { ModOption } from './bindings/ModOption'
import type { Plan } from './bindings/Plan'
import type { Score } from './bindings/Score'
import type { Sections } from './bindings/Sections'
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
  register: (username: string, password: string, email: string) =>
    invoke<void>('register', { username, password, email }),
  confirmAgreement: (code: string) =>
    invoke<void>('confirm_agreement', { code }),
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
  downloadEngine: (version: string) =>
    invoke<string>('download_engine', { version }),
  stopDownload: () => invoke<void>('stop_download'),
  ring: (user: string) => invoke<void>('ring', { user }),
  addBot: (
    name: string,
    ai: string,
    team: number,
    allyTeam: number,
    colour: number,
  ) => invoke<void>('add_bot', { name, ai, team, allyTeam, colour }),
  removeBot: (name: string) => invoke<void>('remove_bot', { name }),
  setAway: (away: boolean) => invoke<void>('set_away', { away }),
  overlayActive: () => invoke<boolean>('overlay_active'),
  overlayToggle: () => invoke<void>('overlay_toggle'),
  stopGame: () => invoke<boolean>('stop_game'),
  quitAll: () => invoke<void>('quit_all'),
  shutdown: () => invoke<void>('shutdown'),
  isFullscreen: () => invoke<boolean>('is_fullscreen'),
  toggleFullscreen: () => invoke<boolean>('toggle_fullscreen'),
  startBoxes: (teams: number) =>
    invoke<BoxesView | null>('start_boxes', { teams }),
  decodeBoxes: (raw: string, teams: number) =>
    invoke<[number, number][][] | null>('decode_boxes', { raw, teams }),
  flashEngine: () => invoke<boolean>('flash_engine'),
  requestGameStatus: (founder: string) =>
    invoke<void>('request_game_status', { founder }),

  pveScore: () => invoke<Score | null>('pve_score'),

  // ---- saved room setups ----
  gameModOptions: (game: string) =>
    invoke<ModOption[]>('game_modoptions', { game }),
  listPresets: () => invoke<Book>('list_presets'),
  chobbyPresetsPath: () => invoke<string | null>('chobby_presets_path'),
  savePreset: (name: string) => invoke<Book>('save_preset', { name }),
  presetFromReplay: (path: string, name: string) =>
    invoke<Book>('preset_from_replay', { path, name }),
  deletePreset: (name: string) => invoke<Book>('delete_preset', { name }),
  renamePreset: (from: string, to: string) =>
    invoke<Book>('rename_preset', { from, to }),
  planPreset: (name: string, sections: Sections) =>
    invoke<Plan>('plan_preset', { name, sections }),
  applyPreset: (name: string, sections: Sections) =>
    invoke<Plan>('apply_preset', { name, sections }),
  importPresets: (path: string | null) =>
    invoke<{ book: Book; skipped: number }>('import_presets', { path }),
  exportPresets: (path: string | null, names: string[]) =>
    invoke<number>('export_presets', { path, names }),
  rememberPlayed: (played: boolean) =>
    invoke<Settings>('remember_played', { played }),
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
