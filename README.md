# modlobby

A Beyond All Reason lobby focused on modding and performance.

## Rust workspace

| Crate             | Role                                                                                                                                            |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `spring-protocol` | Legacy SpringLobbyProtocol (teiserver dialect): line codec, typed events, `LOGIN`/telemetry encoding, throttle policy, TLS-capable transport actor |
| `lobby-core`      | Authoritative client state, the pure reducer `(state, event) -> effects`, and the SPADS announcement parser (votes, setting changes)              |
| `lobby-ui`        | The UI contract: snapshot/delta types (exported to TypeScript by ts-rs), projection from core events, batching, the `UiTransport` seam            |
| `lobby-runtime`   | The tokio actor every front end drives: connection, reducer, engine child, UI transport                                                          |
| `settings`        | User settings as JSONC with comments preserved, live reload, a JSON Schema, and credentials in the OS keyring                                     |
| `tweaks`          | `tweakdefs`/`tweakunits`: base64url, StyLua formatting, minification, `!bSet` commands with the 16 385-character gauge, diffs                     |
| `content`         | What this machine has installed: engines, games via the rapid index, maps — the honest source of the sync bit |
| `modoptions`      | BAR's modoption schema, parsed out of the game's own `modoptions.lua` and vendored as JSON for the app          |
| `presets`         | Saved room setups with timestamps, the plan for applying one, and interop both ways with Chobby's `optionsPresets.json` |
| `startbox`        | Startbox arrangements: the `base64url(zlib(json))` modoptions, and the resolution order the game enforces        |
| `pve`             | What a PvE room scores, from the service BAR's in-game PvE Stats widget uses                                    |
| `recoil`          | Engine launch: `spring://` URL, engine discovery in the BAR data dir, `--write-dir --isolation` command                                          |
| `modlobby-cli`    | Harness: log in as a Chobby-class client, watch the battle list, spectate a room, launch the engine                                              |
| `modlobby-app`    | The Tauri 2 shell (`app/src-tauri`) over `lobby-runtime`; the SolidJS front end lives in `app/`                                                  |

The app covers: the battle list with filters, sorting, map thumbnails and a
hover card naming who is already in a room; the battle room with its minimap,
start boxes, tweaks and settings; channels and private messages with name
completion, recall of what you sent, clickable links, and a mark on every line
that says your name; friends, and a search over everyone online; content
fetched through pr-downloader the moment you join a room that needs it;
a replay browser, whose replays can be turned back into presets; and a skirmish
against AI that needs no server at all. A PvE room shows its challenge score
before anyone plays it.
Background notifications cover direct messages, mentions, votes, rings, a
friend arriving, and your game starting.

```sh
cargo test
cp .env.example .env                                                        # MODLOBBY_USERNAME / MODLOBBY_PASSWORD, for the CLI only
cargo run -p modlobby-cli -- login                                          # TLS to server4.beyondallreason.info:8201; --plain for 8200
cargo run -p modlobby-cli -- join --battle <id> --launch                    # spectate a room; connects the engine while its game runs
cargo run -p modlobby-cli -- policy > policy.toml                           # dump the default throttle policy to tune with --policy
cargo run -p content --example read_game_file -- <data dir> "<game version>"  # the modoption table this machine would read
```

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

## Desktop app

```sh
cd app
bun install
bun run dev                                                                 # Tauri window + Vite
bun run check                                                               # prettier, tsc, vitest
bun run test:watch                                                          # vitest, watching
```

A lobby is only honest against a real server, and several bugs the tests were
happy with turned up only there — an empty channel list saved over a good one,
a settings watcher that had already been dropped. `scripts/webview.ts` drives
the running window over the DevTools protocol so those checks can be made
without a pair of hands:

```sh
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 bun run dev
bun scripts/webview.ts eval "document.querySelectorAll('.battle-row').length"
bun scripts/webview.ts shot battles.png
```

Toolchains are pinned exactly: `rust-toolchain.toml` for Rust, `mise.toml` for the Node the
JS tooling needs. mise honours `rust-toolchain.toml` only when `rust` is listed in
`idiomatic_version_file_enable_tools`; without it a global `[tools] rust` silently wins over
the project pin.

The app keeps its settings in `%APPDATA%\modlobby\config\settings.jsonc` — JSONC with a
schema next to it, so an editor completes the keys and your comments survive the app's own
writes; edits made while it runs are picked up live. Passwords go to the OS keyring, never
to that file. TypeScript types in `app/src/ipc/bindings/` are generated from Rust on
`cargo test`; do not edit them by hand.

You join a room as a spectator and sit down when you want to play. `play.inPublicRooms`,
under Settings → Advanced, turns the seats off for a session that is only watching — a
client driving the protocol with nobody at the keyboard; a room of your own never consults
it. For a room of your own, ask a cluster manager: the app's "Private room" button sends
`!privatehost` and joins the room it opens.

Toolchain is pinned in `rust-toolchain.toml`; dependency versions are exact.

## License

MIT — see [LICENSE](LICENSE). It covers everything in this repository except `external/`.

The `external/` submodules are reference material, not dependencies. Nothing from them is
linked or redistributed: the one `include_str!` of BAR's `modoptions.lua`
(`crates/modoptions/src/lib.rs`) is inside `#[cfg(test)]`, so it exists in the test binary
and never in a shipped one. Each submodule stays under whatever license its upstream
carries, and `scripts/setup-submodules.sh` fetches them from upstream rather than vendoring
copies here.

## Releases and updates

The version is `Cargo.toml`'s, once. `tauri.conf.json` and `app/package.json` carry none;
the corner of the nav shows it as `0.1.1+ad39005`, the commit hash stamped by
`app/src-tauri/build.rs` at build time — a commit cannot contain its own hash, so it is
never a field in a committed file.

A release is one button: **Actions → release → Run workflow**. Blank input bumps the patch
version; a typed one is used as given. The workflow commits the bump, tags `v<version>`,
builds and signs the NSIS installer, and attaches it with its `.sig` and `latest.json` to a
GitHub release. `git checkout v0.1.1 && cargo build` reproduces the shipped version string.

The app fetches `releases/latest/download/latest.json` on startup (Settings → Advanced →
Updates, on by default) and restarts into a newer version before anyone has logged in. Found
while a room is joined or a game is running, the download waits in the nav for a click.

The signing key lives outside the repo in `~/.tauri/modlobby.key` with its password beside
it, and in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. **Back it up.** Without it no installed copy can ever
update again. With the public key in `tauri.conf.json`, a local `bun run build` needs
`TAURI_SIGNING_PRIVATE_KEY_PATH` and the password in the environment; `bun run dev` does not.
