#!/usr/bin/env bun
/**
 * `tauri dev`'s runner, in place of `cargo run`: cargo builds, and this
 * starts the app.
 *
 * `cargo run` keeps what it starts in a Windows job that kills everything in
 * it when cargo goes, and tauri stops cargo to restart the app on every Rust
 * edit -- so a game started from the dev lobby died with it. Bun's own job
 * ends only what bun started and lets their children go (kill-on-close with
 * silent breakaway, seen 2026-09-30 on bun 1.4.0): the app goes with this
 * runner, the engine the app started plays on, and the lobby that comes back
 * takes it over. Where there is no such job the app watches for this runner
 * itself (`MODLOBBY_DEV_RUNNER`).
 *
 * Tauri calls it from `app/src-tauri` as `<runner> run [cargo args] --
 * [app args]`, so `bun run` is the command and this file its first argument;
 * see `app/src-tauri/tauri.dev.json`. A failed build is read off stderr as
 * "could not compile", which cargo still writes there.
 */

type Artifact = {
  reason: string
  executable?: string | null
  target?: { kind: string[] }
}

const args = process.argv.slice(2)
const split = args.indexOf('--')
const cargoArgs = split === -1 ? args : args.slice(0, split)
const appArgs = split === -1 ? [] : args.slice(split + 1)

// Diagnostics rendered to stderr as usual; stdout carries the one fact
// needed here, where the binary was written.
const build = Bun.spawn(
  ['cargo', 'build', '--message-format=json-render-diagnostics', ...cargoArgs],
  { stdout: 'pipe', stderr: 'inherit' },
)
const messages = await new Response(build.stdout).text()
const built = await build.exited
if (built !== 0) process.exit(built)

const binary = messages
  .split('\n')
  .filter((line) => line.startsWith('{'))
  .map((line) => JSON.parse(line) as Artifact)
  .findLast(
    (message) =>
      message.reason === 'compiler-artifact' &&
      !!message.executable &&
      !!message.target?.kind.includes('bin'),
  )?.executable
if (!binary) {
  console.error('dev-runner: cargo built no binary to run')
  process.exit(1)
}

const app = Bun.spawn([binary, ...appArgs], {
  stdio: ['inherit', 'inherit', 'inherit'],
  env: { ...process.env, MODLOBBY_DEV_RUNNER: String(process.pid) },
})
process.exit(await app.exited)
