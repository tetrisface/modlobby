import { invoke, type Channel } from '@tauri-apps/api/core'
import type { Act } from './bindings/Act'
import type { AiChoice } from './bindings/AiChoice'
import type { Arrangement } from './bindings/Arrangement'
import type { ArrangementView } from './bindings/ArrangementView'
import type { Check } from './bindings/Check'
import type { Encoded } from './bindings/Encoded'
import type { DefTags } from './bindings/DefTags'
import type { DiffView } from './bindings/DiffView'
import type { Kind } from './bindings/Kind'
import type { MapIndex } from './bindings/MapIndex'
import type { NewsFeed } from './bindings/NewsFeed'
import type { Prepared } from './bindings/Prepared'
import type { ReplayView } from './bindings/ReplayView'
import type { SkirmishOptions } from './bindings/SkirmishOptions'
import type { BoxesView } from './bindings/BoxesView'
import type { Book } from './bindings/Book'
import type { ModOption } from './bindings/ModOption'
import type { Plan } from './bindings/Plan'
import type { PlayerFilesView } from './bindings/PlayerFilesView'
import type { Score } from './bindings/Score'
import type { Sections } from './bindings/Sections'
import type { Settings } from './bindings/Settings'
import type { Tile } from './bindings/Tile'
import type { Slot } from './bindings/Slot'
import type { TweakView } from './bindings/TweakView'
import type { UiMessage } from './bindings/UiMessage'
import type { UpdateProgress } from './bindings/UpdateProgress'
import type { VersionView } from './bindings/VersionView'

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
  reconnect: () => invoke<void>('reconnect'),
  /** Answers with the user agreement the server replies to the first login with. */
  register: (username: string, password: string, email: string) =>
    invoke<string[]>('register', { username, password, email }),
  /** Finishes that login; the account is what this machine remembers after. */
  confirmAgreement: (
    username: string,
    password: string,
    code: string,
    remember: boolean,
    autoLogin: boolean,
  ) =>
    invoke<void>('confirm_agreement', {
      username,
      password,
      code,
      remember,
      autoLogin,
    }),
  /** Why a username would be refused, without spending a round trip on it. */
  nameProblem: (username: string) =>
    invoke<string | null>('name_problem', { username }),
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
  /** Looks at what is installed again, for an engine that arrived by hand. */
  recheckContent: () => invoke<void>('recheck_content'),
  mapIndex: () => invoke<MapIndex>('map_index'),
  warmMapPictures: (maps: string[], tiles: Tile[]) =>
    invoke<void>('warm_map_pictures', { maps, tiles }),

  // ---- news ----
  /** The feed and how much of it is new, in one answer so the two agree. */
  news: () => invoke<NewsFeed>('news'),
  markNewsRead: () => invoke<void>('mark_news_read'),

  downloadEngine: (version: string) =>
    invoke<string>('download_engine', { version }),
  stopDownload: () => invoke<void>('stop_download'),
  cancelPaste: () => invoke<void>('cancel_paste'),
  appVersion: () => invoke<VersionView>('app_version'),
  checkUpdate: () => invoke<UpdateProgress>('check_update'),
  installUpdate: () => invoke<UpdateProgress>('install_update'),
  /** Installs a download an earlier run kept; `null` when there is none. */
  resumeUpdate: () => invoke<UpdateProgress | null>('resume_update'),
  ring: (user: string) => invoke<void>('ring', { user }),
  addBot: (
    name: string,
    ai: string,
    team: number,
    allyTeam: number,
    colour: number,
  ) => invoke<void>('add_bot', { name, ai, team, allyTeam, colour }),
  /** Only for an AI we added: the server drops anyone else's, silently. */
  updateBot: (
    name: string,
    team: number,
    allyTeam: number,
    handicap: number,
    colour: number,
  ) => invoke<void>('update_bot', { name, team, allyTeam, handicap, colour }),
  removeBot: (name: string) => invoke<void>('remove_bot', { name }),
  setAway: (away: boolean) => invoke<void>('set_away', { away }),
  activity: () => invoke<void>('activity'),
  overlayActive: () => invoke<boolean>('overlay_active'),
  overlayToggle: () => invoke<void>('overlay_toggle'),
  overlayPainted: () => invoke<void>('overlay_painted'),
  stopGame: () => invoke<boolean>('stop_game'),
  quitAll: () => invoke<void>('quit_all'),
  shutdown: () => invoke<void>('shutdown'),
  isFullscreen: () => invoke<boolean>('is_fullscreen'),
  toggleFullscreen: () => invoke<boolean>('toggle_fullscreen'),
  startBoxes: (teams: number) =>
    invoke<BoxesView | null>('start_boxes', { teams }),
  decodeBoxes: (raw: string, teams: number) =>
    invoke<[number, number][][] | null>('decode_boxes', { raw, teams }),
  currentArrangement: (teams: number) =>
    invoke<ArrangementView | null>('current_arrangement', { teams }),
  encodeBoxes: (arrangement: Arrangement) =>
    invoke<Encoded>('encode_boxes', { arrangement }),
  describeMapOption: (key: string, raw: string) =>
    invoke<string>('describe_map_option', { key, raw }),
  flashEngine: () => invoke<boolean>('flash_engine'),
  requestGameStatus: (founder: string) =>
    invoke<void>('request_game_status', { founder }),

  pveScore: () => invoke<Score | null>('pve_score'),

  // ---- saved room setups ----
  gameModOptions: (game: string) =>
    invoke<ModOption[]>('game_modoptions', { game }),
  gameAis: (game: string) => invoke<AiChoice[]>('game_ais', { game }),
  /** What an engine AI declares it can be told; empty when it declares none. */
  aiOptions: (engine: string, ai: string) =>
    invoke<ModOption[]>('ai_options', { engine, ai }),
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

  // ---- the room with no server behind it ----
  /** Opens it, on the newest of whatever this machine has unless told otherwise. */
  skirmishOpen: (
    game: string | null,
    map: string | null,
    engine: string | null,
  ) => invoke<void>('skirmish_open', { game, map, engine }),
  skirmishClose: () => invoke<void>('skirmish_close'),
  /** One change to it. Every way it can change goes through here. */
  skirmishAct: (act: Act) => invoke<void>('skirmish_act', { act }),
  skirmishLaunch: () => invoke<void>('skirmish_launch'),
  /**
   * Joins a game somebody on this network is hosting.
   *
   * By the announcement's id rather than by address: a game that stopped
   * being announced between the list being drawn and the click is refused,
   * rather than joined at an address that may since be somebody else's.
   */
  joinLanGame: (id: string, asName?: string) =>
    invoke<void>('join_lan_game', { id, asName: asName ?? null }),
  skirmishDownloadMissing: () => invoke<void>('skirmish_download_missing'),
  skirmishStartBoxes: (teams: number) =>
    invoke<BoxesView | null>('skirmish_start_boxes', { teams }),
  skirmishCurrentArrangement: (teams: number) =>
    invoke<ArrangementView | null>('skirmish_current_arrangement', { teams }),
  skirmishPveScore: () => invoke<Score | null>('skirmish_pve_score'),
  skirmishSavePreset: (name: string) =>
    invoke<Book>('skirmish_save_preset', { name }),
  skirmishApplyPreset: (name: string, sections: Sections) =>
    invoke<Plan>('skirmish_apply_preset', { name, sections }),
  skirmishTweakSend: (lua: string, slot: Slot, direct: boolean) =>
    invoke<Prepared>('skirmish_tweak_send', { lua, slot, direct }),
  skirmishTweakClear: (slot: Slot) =>
    invoke<void>('skirmish_tweak_clear', { slot }),
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
  openEngineDir: () => invoke<void>('open_engine_dir'),
  playerFiles: () => invoke<PlayerFilesView>('player_files'),
  importPlayerFiles: (from: string) =>
    invoke<number>('import_player_files', { from }),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  openLogDir: () => invoke<void>('open_log_dir'),

  takeSeat: (team: number, allyTeam: number) =>
    invoke<void>('take_seat', { team, allyTeam }),
  releaseSeat: () => invoke<void>('release_seat'),
  setReady: (ready: boolean) => invoke<void>('set_ready', { ready }),
  setSide: (side: number) => invoke<void>('set_side', { side }),
  requestPrivateHost: () => invoke<string>('request_private_host'),
  hostPublic: () => invoke<number>('host_public'),

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
  tweakCheck: (lua: string, kind: Kind) =>
    invoke<Check>('tweak_check', { lua, kind }),
  tweakDiffText: (kind: Kind, left: string, right: string) =>
    invoke<DiffView>('tweak_diff_text', { kind, left, right }),
  gameUnitNames: (game: string) =>
    invoke<string[]>('game_unit_names', { game }),
  engineDefTags: (version: string) =>
    invoke<DefTags>('engine_def_tags', { version }),
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

/** The `code` a failed command rejected with; null when it was not one of ours. */
export function errorCode(error: unknown): string | null {
  if (typeof error === 'object' && error !== null && 'code' in error) {
    return String((error as ApiError).code)
  }
  return null
}
