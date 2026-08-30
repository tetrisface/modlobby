/**
 * Three dots, one lit at a time, while something is being asked for.
 *
 * Deliberately small and quiet. Its job is only to stop an answer appearing
 * out of nowhere — a number that fades in unannounced reads as a glitch, and a
 * spinner in a table row reads as a problem.
 *
 * The cycle is CSS, so a list of these costs no timers and keeps step with
 * each other.
 */
export function Thinking(props: { title?: string }) {
  return (
    <span class='thinking' title={props.title ?? 'asking…'} aria-label='asking'>
      <i />
      <i />
      <i />
    </span>
  )
}
