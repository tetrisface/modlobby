#!/usr/bin/env bun
/**
 * Names the GitHub Actions in .github/workflows whose pinned commit is not
 * their latest release's, with the line that pins that release. Dependabot
 * raises no alerts for SHA-pinned actions and opens no PRs here, so `just dev`
 * runs this instead. A check that cannot run says so in one line; it never
 * stops the caller.
 *
 * Commits are compared, not tags, because tags name releases unevenly:
 * tauri-action's `v1.0.0` is its `action-v1.0.0`. One GraphQL request asks for
 * every action's latest release at once.
 */

const WORKFLOWS = `${import.meta.dir}/../.github/workflows`
const PIN = /uses: ([\w.-]+)\/([\w.-]+)[\w./-]*@([0-9a-f]{40})(?: # (\S+))?/g
const TIMEOUT_MS = 3000

type Pin = { owner: string; name: string; sha: string; version: string }
type Release = { tagName: string; tagCommit: { oid: string } | null } | null
type Answer = {
  data?: Record<string, { latestRelease: Release } | null>
  errors?: { message: string }[]
}

function githubToken(): string | undefined {
  const fromEnv = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN
  if (fromEnv) return fromEnv
  try {
    const gh = Bun.spawnSync(['gh', 'auth', 'token'], { stderr: 'ignore' })
    return gh.success ? gh.stdout.toString().trim() : undefined
  } catch {
    // gh is not installed.
    return undefined
  }
}

async function readPins(): Promise<Pin[]> {
  const pins = new Map<string, Pin>()
  for await (const file of new Bun.Glob('*.yml').scan(WORKFLOWS)) {
    const workflow = await Bun.file(`${WORKFLOWS}/${file}`).text()
    for (const [, owner, name, sha, version = sha.slice(0, 7)] of workflow.matchAll(PIN)) {
      pins.set(`${owner}/${name}@${sha}`, { owner, name, sha, version })
    }
  }
  return [...pins.values()]
}

async function latestReleases(pins: Pin[], token: string): Promise<Release[]> {
  const fields = pins.map(
    (pin, index) =>
      `r${index}: repository(owner: ${JSON.stringify(pin.owner)}, name: ${JSON.stringify(pin.name)}) ` +
      '{ latestRelease { tagName tagCommit { oid } } }',
  )
  const response = await fetch('https://api.github.com/graphql', {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}` },
    body: JSON.stringify({ query: `{ ${fields.join(' ')} }` }),
    signal: AbortSignal.timeout(TIMEOUT_MS),
  })
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
  const answer = (await response.json()) as Answer
  if (answer.errors?.length) throw new Error(answer.errors.map((error) => error.message).join('; '))
  return pins.map((_, index) => answer.data?.[`r${index}`]?.latestRelease ?? null)
}

function describe(pin: Pin, release: Release): string[] {
  const repo = `${pin.owner}/${pin.name}`
  if (!release?.tagCommit) return [`  ${repo}: no release to compare with`]
  if (release.tagCommit.oid === pin.sha) return []
  return [
    `  ${repo} ${pin.version} -> ${release.tagName}`,
    `    uses: ${repo}@${release.tagCommit.oid} # ${release.tagName}`,
  ]
}

async function report(token: string): Promise<string> {
  const pins = await readPins()
  const releases = await latestReleases(pins, token)
  const lines = pins.flatMap((pin, index) => describe(pin, releases[index]))
  return lines.length > 0 ? `Pinned actions behind their latest release:\n${lines.join('\n')}` : ''
}

const token = githubToken()
if (!token) {
  console.log('Action pins not checked: no GitHub token (`gh auth login`, or GH_TOKEN)')
  process.exit(0)
}
try {
  const text = await report(token)
  if (text) console.log(text)
} catch (error) {
  console.log(`Action pins not checked: ${error instanceof Error ? error.message : String(error)}`)
}
