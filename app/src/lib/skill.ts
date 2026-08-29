/**
 * Reading a player's skill out of the room's script tags.
 *
 * `sendPlayerSkill` (`SPADS/src/spads.pl:6366`) wraps the number in a sigil
 * naming where it came from, so a parser that reads the value directly gets
 * `NaN` for BAR. It also only sends a *real* sigma for plugin-provided skill:
 * TrueSkill's uncertainty is bucketed to 0-3 before it goes on the wire. BAR
 * runs a plugin, so we get `[14.61]` with a raw sigma beside it.
 */

export type SkillOrigin =
  | 'exact'
  | 'rounded'
  | 'degraded'
  | 'plugin'
  | 'pluginDegraded'
  | 'rank'
  | 'unknown'

export type Skill = {
  value: number
  origin: SkillOrigin
  /** Raw sigma for plugin skill; a 0-3 bucket for TrueSkill; absent otherwise. */
  sigma: number | null
}

/** Longest sigil first: `[#n#]` also matches the `[n]` pattern. */
const SIGILS: ReadonlyArray<readonly [RegExp, SkillOrigin]> = [
  [/^\[#(.*)#\]$/, 'pluginDegraded'],
  [/^\[(.*)\]$/, 'plugin'],
  [/^#(.*)#$/, 'degraded'],
  [/^\((.*)\)$/, 'rank'],
  [/^\?(.*)\?$/, 'unknown'],
  [/^~(.*)$/, 'rounded'],
]

/** Past this, Chobby stops stating the number at all. */
const UNRATED_SIGMA = 6.65

/** Chobby's `skillUncertaintyDistribution`, brightest first. */
const TIERS = [1.5, 2, 3]

export function parseSkill(
  raw: string | undefined,
  uncertainty: string | undefined,
): Skill | null {
  if (raw === undefined) return null

  const text = raw.trim()
  const matched = SIGILS.find(([pattern]) => pattern.test(text))
  const digits = matched ? text.replace(matched[0], '$1').trim() : text
  // `Number('')` is 0, which would turn an empty tag into a real rating.
  if (digits === '') return null

  const value = Number(digits)
  if (!Number.isFinite(value)) return null

  const sigma = uncertainty === undefined ? NaN : Number(uncertainty)
  return {
    value,
    origin: matched ? matched[1] : 'exact',
    sigma: Number.isFinite(sigma) ? sigma : null,
  }
}

/** Skill by lowercased player name — the case SPADS sends its tags in. */
export function readSkills(
  scriptTags: Record<string, string> | undefined,
): Record<string, Skill> {
  const skills: Record<string, Skill> = {}
  if (!scriptTags) return skills

  for (const [key, raw] of Object.entries(scriptTags)) {
    const name = /^game\/players\/(.+)\/skill$/.exec(key)?.[1]
    if (name === undefined) continue
    const skill = parseSkill(
      raw,
      scriptTags[`game/players/${name}/skilluncertainty`],
    )
    if (skill) skills[name] = skill
  }
  return skills
}

/** `??` is "too uncertain to state", not "unknown". */
export function isUnrated(skill: Skill): boolean {
  return skill.sigma !== null && skill.sigma > UNRATED_SIGMA
}

export function skillText(skill: Skill): string {
  return isUnrated(skill) ? '??' : String(Math.round(Math.max(0, skill.value)))
}

/** Brightness carries confidence: a dim number is one to trust less. */
export function skillTier(skill: Skill): 0 | 1 | 2 | 3 {
  if (skill.sigma === null) return 0
  const sigma = skill.sigma
  return TIERS.filter((tier) => sigma >= tier).length as 0 | 1 | 2 | 3
}

const ORIGINS: Record<SkillOrigin, string> = {
  exact: 'OpenSkill',
  rounded: 'OpenSkill, rounded',
  degraded: 'OpenSkill, degraded',
  plugin: 'OpenSkill',
  pluginDegraded: 'OpenSkill, degraded',
  rank: 'estimated from lobby rank',
  unknown: 'OpenSkill, unknown origin',
}

export function skillTitle(skill: Skill): string {
  const sigma = skill.sigma === null ? '' : ` (σ=${skill.sigma})`
  return `${ORIGINS[skill.origin]}: ${skill.value}${sigma}`
}

/** Only players with a skill count; an unrated player contributes nothing. */
export function teamSkill(skills: Array<Skill | null>): number {
  return skills.reduce((total, skill) => total + (skill?.value ?? 0), 0)
}
