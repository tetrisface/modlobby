import { Navigate } from '@solidjs/router'
import { Show } from 'solid-js'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'

/**
 * Where a launch lands: the lobby when there is going to be a session, and a
 * skirmish when there is not.
 *
 * A game against AI needs no account and no server, so somebody with no
 * account has something to play rather than a form to fill in — the login page
 * is somewhere you go, not a wall you pass. But a launch that is about to log
 * itself in belongs in the battle list: that is where the offer to rejoin the
 * room you were in last shows up, and it would be missed on another page.
 *
 * Decided once, on the way through, rather than by watching the phase. An
 * effect that moved you to the list the moment a session arrived would also
 * move you mid-reconnect, out of the skirmish you were half-way through
 * setting up.
 *
 * Nothing is drawn until the settings arrive, which is one file read away and
 * already in flight: the shell is on screen throughout, so this is a frame or
 * two of empty page rather than a wait.
 */
export function Home() {
  return (
    <Show when={settings()}>
      {(loaded) => {
        const account = loaded().account
        const expectSession =
          lobby.phase === 'ready' ||
          (account.autoLogin && account.rememberPassword)
        return <Navigate href={expectSession ? '/battles' : '/skirmish'} />
      }}
    </Show>
  )
}
