# modlobby

A Beyond All Reason lobby focused on modding, experiments and performance.

# Install

## Packages

Packages are available at [releases](https://github.com/tetrisface/modlobby/releases) as .exe,
.AppImage, .deb and .rpm, all of which update themselves from the next release. MacOS is not supported for skirmish yet.

## From source

Needs Rust via [rustup](https://rustup.rs) (the toolchain is pinned by `rust-toolchain.toml`),
[bun](https://bun.sh), and Tauri 2's [system prerequisites](https://v2.tauri.app/start/prerequisites/)
for your platform. [mise](https://mise.jdx.dev/) installs the pinned node and bun from `mise.toml`.

```sh
git clone https://github.com/tetrisface/modlobby
cd modlobby/app
bun install
bun run build    # installer under app/src-tauri/target/release/bundle/
```

# Alongside other lobbies

modlobby is built to sit next to Chobby and bar-lobby on the same machine rather
than to replace them, and the arrangement is deliberate: it reads their installs
so nothing is downloaded twice, and it writes almost nothing back. The one rule
behind all of it is that a half-finished download of ours can never appear in
their view of the world, and theirs can at worst be missing from ours.

## Where things are

| Directory              | Windows                                          | Linux                                                            | modlobby         |
| ---------------------- | ------------------------------------------------ | ---------------------------------------------------------------- | ---------------- |
| Its own content        | `%LOCALAPPDATA%\modlobby\data`                   | `~/.local/share/modlobby/data`                                   | reads and writes |
| Its own settings       | `%APPDATA%\modlobby\config`                      | `~/.config/modlobby`                                             | reads and writes |
| The launcher's install | `%LOCALAPPDATA%\Programs\Beyond-All-Reason\data` | `$XDG_STATE_HOME/Beyond-All-Reason`                              | reads            |
| bar-lobby's assets     | `%LOCALAPPDATA%\Programs\BeyondAllReason\assets` | `$BAR_ASSETS_PATH`, else `$XDG_DATA_HOME/BeyondAllReason/assets` | reads            |

Engines, games, maps, replays and even pr-downloader itself are taken from
whichever of those directories already has them, so a machine that has played
BAR before needs no second copy of anything. Everything modlobby fetches goes
into its own directory. Setting `paths.dataDir` points the writing somewhere
else — at another lobby's install, if that is what you want — and the reading is
unchanged either way.

## What it writes where another lobby can see it

Two things, and only two.

The first is **presets**, and only when you ask. Chobby keeps its own in
`optionsPresets.json`; modlobby keeps `presets.json` in its config directory and
interoperates with that file in both directions. Export writes to wherever a
Chobby `optionsPresets.json` already exists, backing up what was there as
`optionsPresets.json.modlobby.bak` first. Import never overwrites a preset of
yours that has the same name — it skips it. Nothing syncs by itself; the file is
the hand-over.

The second is a pair of **generated files for the running game**, both written
on start and removed when modlobby exits: `LuaUI/Widgets/modlobby_escape.lua`,
and a small LuaMenu archive under `games/`. It goes in the directory
modlobby writes, which is its own unless you have changed `paths.dataDir`. It
draws nothing, and it is inert unless modlobby is listening on the loopback port
baked into it — so a game launched from Chobby behaves exactly as it always did,
including keeping its own Escape. Turn it off with the Escape setting under
Overlay.

The menu archive exists for one reason: BAR's in-game top bar offers a **Lobby**
button in place of **Quit** when the engine was started with a menu whose name
contains `chobby` (`gui_top_bar.lua:3574`), and that button leaves the game
running and asks the menu to show itself — which is exactly modlobby's overlay.
The archive has no interface of its own and never draws; it turns that one
message into the same request the Escape widget sends, and quits the process
when a finished game drops back into it. It is a menu rather than a game
(`modtype = 5`, `onlyLocal`), so it does not appear in anyone's game list, and
modlobby passes `--menu` only when the archive is actually on disk — a name the
engine cannot resolve stops it starting at all. Turning the overlay off turns
this off with it.

## What is not shared, though it looks as if it should be

`springsettings.cfg` is **never written** — not by modlobby, and not by the game
modlobby starts. When the overlay needs a borderless window and your settings say
exclusive full screen, the engine is launched with `--config` pointed at a private
copy under `%APPDATA%\modlobby\config\engine\`, which is exclusive
(`ConfigHandler.cpp:420`), so the file Chobby reads is left byte for byte as you
left it. The corollary is that graphics settings you change _inside_ a
modlobby-launched game land in that private copy and do not reach Chobby.

`uikeys.txt` and `LuaUI/Config/` — your keybinds, and which widgets you have
enabled with their own stored data — are copied **once**, when modlobby's data
directory is new and another install has a healthy set. After that they are two
separate files that drift apart: a widget you disable in a modlobby game stays
enabled in Chobby. The seeding never overwrites a file that is already there, and
it skips a source whose settings file looks damaged.

Chat logs, the battle-list filters, the notification settings and everything else
in `settings.jsonc` are modlobby's alone. Your password is in the operating
system's keyring under the service name `modlobby`, not in any file and not
shared with Chobby's own stored login.

## What is shared that you might not expect

Games launched from modlobby **do** load the user widgets in other lobbies'
installs. The engine searches every data directory it is given for
`LuaUI/Widgets`, and modlobby passes the other installs in `SPRING_DATADIR` so
that their maps and games are visible (`DataDirsAccess.cpp:45`). Widgets come
along with that. This is usually what you want and is worth knowing when a game
started from modlobby behaves like your Chobby one.

Before every launch, modlobby takes a copy of `springsettings.cfg`, `uikeys.txt`
and `LuaUI/Config/` into `modlobby-backups/` in its write directory, keeping the
last ten. The engine has emptied the settings file before — spring-launcher has
carried a backup against exactly that since 2022 — so if one comes back out of a
game with most of its keys gone, modlobby says so and tells you where the copy
from before the game is. Putting it back is your decision, from Settings.

# Development

After `bun install` above launch the app with `bun run dev`.

## Rust workspace

| Crate             | Role                                                                                                                                                                                         |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `spring-protocol` | Legacy SpringLobbyProtocol (teiserver dialect): line codec, typed events, `LOGIN`/telemetry encoding, throttle policy, TLS-capable transport actor                                           |
| `lobby-core`      | Authoritative client state, the pure reducer `(state, event) -> effects`, and the SPADS announcement parser (votes, setting changes)                                                         |
| `lobby-ui`        | The UI contract: snapshot/delta types (exported to TypeScript by ts-rs), projection from core events, batching, the `UiTransport` seam                                                       |
| `lobby-runtime`   | The tokio actor every front end drives: connection, reducer, engine child, UI transport                                                                                                      |
| `settings`        | User settings as JSONC with comments preserved, live reload, a JSON Schema, and credentials in the OS keyring                                                                                |
| `tweaks`          | `tweakdefs`/`tweakunits`: base64url, StyLua formatting, minification, `!bSet` commands with the 16 385-character gauge, diffs                                                                |
| `content`         | What this machine has installed: engines, games via the rapid index, maps — the honest source of the sync bit; the one named HTTP client, and BAR's map index cached on disk behind its ETag |
| `modoptions`      | BAR's modoption schema, parsed out of the game's own `modoptions.lua` and vendored as JSON for the app                                                                                       |
| `presets`         | Saved room setups with timestamps, the plan for applying one, and interop both ways with Chobby's `optionsPresets.json`                                                                      |
| `startbox`        | Startbox arrangements: the `base64url(zlib(json))` modoptions, and the resolution order the game enforces                                                                                    |
| `skirmish`        | A battle room with no server behind it: the same room the lobby draws, changed locally, and the start script it becomes                                                                      |
| `pve`             | PvE room difficulty estimate from the 3rd-party service pve.bar.                                                                                                                             |
| `recoil`          | Engine launch: `spring://` URL, engine discovery in the BAR data dir, `--write-dir --isolation` command                                                                                      |
| `modlobby-cli`    | Harness: log in as a Chobby-class client, watch the battle list, spectate a room, launch the engine                                                                                          |
| `modlobby-app`    | The Tauri 2 shell (`app/src-tauri`) over `lobby-runtime`; the SolidJS front end lives in `app/`                                                                                              |

The app covers: the battle list with filters, sorting, map thumbnails and a
hover card naming who is already in a room; the battle room with its minimap,
start boxes, tweaks and settings; channels and private messages with name
completion, recall of what you sent, clickable links, and a mark on every line
that says your name; friends, and a search over everyone online; content
fetched through pr-downloader the moment you join a room that needs it;
a replay browser, whose replays can be turned back into presets; and a skirmish
against AI that is the same room with nobody else in it — teams, settings,
tweaks, start boxes and presets, all without a server. A PvE room shows its
challenge score before anyone plays it.
Background notifications cover direct messages, mentions, votes, rings, a
friend arriving, and your game starting.

### Modoptions

Chobby reads `modoptions.lua` out of the game archive with the engine's Lua VM. modlobby has no
VM, so the `modoptions` crate parses that table with `full_moon` — and it parses the copy already
installed on this machine, read straight out of rapid's package index and content pool by
`content::Library::game_file`.

Nothing is vendored. The names and descriptions in that file are BAR's writing under GPL v2, every
player already has it, and a lobby has no reason to redistribute a copy; `bar-lobby` reads it the
same way. The table therefore also matches whatever game version the room is running, rather than
whatever was current when someone last refreshed a checked-in JSON file.

The room's Setup pane renders BAR's own sections, in BAR's weight order, grouped by BAR's own
`-- Name` subheaders, and opens on the settings that differ from their declared default. The
one arrangement that is ours is a **Modding** tab: the twenty tweak slots plus the six options
that decide which unit definitions exist (`forceallunits`, the Legion faction, and the two
unit packs). `section` is a lobby display hint by BAR's own description, so regrouping changes
nothing on the wire, and Cheats keeps its name and every balance setting.

## Extras

Beyond All Reason publishes no engine for MacOS, and the Apple Silicon build that exists —
[RecoilEngine-AppleSilicon](https://github.com/Vandomas/RecoilEngine-AppleSilicon) — has online
play turned off, because unofficial builds are not permitted on the official servers until their
author has approval and lobby integration is done.

`scripts/webview.ts` drives the running window over the DevTools protocol so those checks can be
made without a pair of hands:

```sh
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 bun run dev
bun scripts/webview.ts eval "document.querySelectorAll('.battle-row').length"
bun scripts/webview.ts shot battles.png
```

The app keeps its settings in `%APPDATA%\modlobby\config\settings.jsonc` on Windows and
`~/.config/modlobby/settings.jsonc` on Linux — JSONC with a
schema next to it, so an editor completes the keys and your comments survive the app's own
writes; edits made while it runs are picked up live. Passwords go to the OS keyring, never
to that file. TypeScript types in `app/src/ipc/bindings/` are generated from Rust on
`cargo test`; do not edit them by hand.

Engines, games and maps it downloads go to a data directory of its own:
`%LOCALAPPDATA%\modlobby\data` on Windows, `~/.local/share/modlobby/data` on Linux. The
launcher's and bar-lobby's installs are read alongside it and never written, so content they
already have is not fetched twice and nothing half-downloaded of ours ends up in their tree.
`paths.dataDir` moves the write directory, for instance onto an existing install.

The files in it that hold what you set up — `springsettings.cfg`, `uikeys.txt` and
`LuaUI/Config/` (widget order, enabled state and each widget's own data) — are copied from
another install the first time the directory is used, snapshotted under `modlobby-backups/`
before every launch (the last ten kept), and checked when the engine exits: it rewrites the
settings file on its way out and has emptied it before. Settings → Paths copies them from an
install or puts a snapshot back.

Toolchains are pinned exactly: `rust-toolchain.toml` for Rust, `mise.toml` for the Node the
JS tooling needs. mise honours `rust-toolchain.toml` only when `rust` is listed in
`idiomatic_version_file_enable_tools`; without it a global `[tools] rust` silently wins over
the project pin.

## License

MIT — see [LICENSE](LICENSE). It covers everything in this repository except `external/`.

The `external/` submodules are reference material, not dependencies. Nothing from them is
linked or redistributed: the one `include_str!` of BAR's `modoptions.lua`
(`crates/modoptions/src/lib.rs`) is inside `#[cfg(test)]`, so it exists in the test binary
and never in a shipped one. Each submodule stays under whatever license its upstream
carries, and `scripts/setup-submodules.sh` fetches them from upstream rather than vendoring
copies here.
