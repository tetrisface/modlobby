import { type Signal, createEffect, createSignal } from 'solid-js'
import { localStore } from './resize'

/**
 * A signal whose value outlives the view it lives in.
 *
 * Toolbar state -- which window, which sort, which filter -- is a preference a
 * player sets once and expects to find again, not something to re-pick every
 * time a route unmounts. Written on every change and read back on mount.
 *
 * Storage that cannot be read, or holds something this build no longer
 * understands, falls back to the default rather than throwing: a remembered
 * filter is a convenience, never a thing the page depends on. A value that is
 * still well-formed JSON but no longer valid -- a sort key a later build
 * renamed -- is the caller's to guard, the same way this page already guards
 * the window and audience it was given.
 */
export function sticky<T>(key: string, fallback: T): Signal<T> {
  const signal = createSignal<T>(read(key, fallback))
  createEffect(() => write(key, signal[0]()))
  return signal
}

function read<T>(key: string, fallback: T): T {
  const held = localStore()?.getItem(key)
  if (!held) return fallback
  try {
    return JSON.parse(held) as T
  } catch {
    return fallback
  }
}

function write<T>(key: string, value: T): void {
  try {
    localStore()?.setItem(key, JSON.stringify(value))
  } catch {
    // A full or locked store costs a remembered preference, nothing more.
  }
}
