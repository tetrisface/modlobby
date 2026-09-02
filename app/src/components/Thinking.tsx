/**
 * Three dots, one lit at a time, while something is being asked for.
 *
 * Deliberately small and quiet. Its job is only to stop an answer appearing
 * out of nowhere — a number that fades in unannounced reads as a glitch, and a
 * spinner in a table row reads as a problem.
 *
 * The cycle is CSS, so a list of these costs no timers and keeps step with
 * each other. `still` keeps the dots but not the motion, for a place someone
 * looks at for minutes at a time — a room panel — where a clock in the
 * corner is noise and three dim dots already say "not yet".
 */
export function Thinking(props: { title?: string; still?: boolean }) {
  return (
    <span
      classList={{ thinking: true, still: props.still }}
      title={props.title ?? 'asking…'}
      aria-label='asking'
    >
      <i />
      <i />
      <i />
    </span>
  )
}
