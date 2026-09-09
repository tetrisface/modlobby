import { createSignal } from 'solid-js'

/**
 * Whether the window is sitting over a running game.
 *
 * Written by the shell, which owns the listener and seeds it by asking -- a
 * webview that reloads mid-overlay was not there for the event. Read wherever
 * something should change with the overlay: the page's modal dress, and a
 * button's doubt, which ought to reset when the game comes back.
 */
export const [over, setOver] = createSignal(false)
