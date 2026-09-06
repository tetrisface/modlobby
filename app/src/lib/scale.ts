/**
 * How large to draw the interface, as a percentage.
 *
 * Every length in `styles.css` is `rem`, and the root font size is
 * `calc(16px * var(--ui-scale))`, so one number here sizes the whole window.
 *
 * The number is remembered per screen, not globally: a laptop and the monitor
 * it is plugged into want different answers, and whichever was used last
 * should not win. Chobby buckets displays for the same reason
 * (`chobby/components/configuration.lua:593`).
 */

/** The smallest and largest we will draw, whatever the file says. */
const FLOOR = 50
const CEILING = 400

/**
 * Which screen this is, coarsely.
 *
 * Rounded hard so that a display which reports a few pixels differently — a
 * taskbar appearing, a scaling change — is still the same display, rather
 * than a new one with no remembered size.
 */
export function bucket(width: number, height: number): string {
  return `${Math.floor(width / 500)}x${Math.floor(height / 250)}`
}

/**
 * The range worth offering on this screen.
 *
 * The ceiling is Chobby's: 200% at 1080p, 400% at 4k — enough that the
 * interface is usable from a sofa, not so much that a window cannot hold a
 * battle list. The floor is where 16px body text reaches 8px and stops being
 * text.
 */
export function bounds(
  width: number,
  height: number,
): { min: number; max: number } {
  const max = Math.round(
    Math.max(100, (width / 960) * 100, (height / 540) * 100),
  )
  return { min: FLOOR, max: Math.min(CEILING, max) }
}

/**
 * What to draw at on a screen nobody has chosen a size for.
 *
 * Screen height in CSS pixels already accounts for the operating system's own
 * scaling, so a tall screen here really does mean physically smaller text.
 * Deliberately gentle: the nav is sized up on every display already, and
 * multiplying that again on a large one overshoots.
 */
export function derived(_width: number, height: number): number {
  if (height >= 1800) return 125
  if (height >= 1300) return 110
  return 100
}

export function clamp(percent: number, width: number, height: number): number {
  const { min, max } = bounds(width, height)
  return Math.min(max, Math.max(min, Math.round(percent)))
}

/** The size to draw at: what was chosen for this screen, else what suits it. */
export function scaleFor(
  saved: Record<string, number>,
  width: number,
  height: number,
): number {
  const chosen = saved[bucket(width, height)]
  return clamp(chosen ?? derived(width, height), width, height)
}

/**
 * The next size up or down, one notch.
 *
 * Notches rather than a factor of the current value, so Ctrl+wheel moves the
 * same amount however far it has already gone, and always lands on a round
 * number a person can read back in settings.
 */
export function step(percent: number, direction: 1 | -1): number {
  return Math.round(percent / 10) * 10 + direction * 10
}
