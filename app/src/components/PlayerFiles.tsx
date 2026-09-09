import { Select } from './Select'
import { For, Show, createResource, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'

/**
 * `2026-09-06T00-48-12Z`, the snapshot directory's name, as a time someone
 * can read. Anything else is shown as it is.
 */
export function snapshotLabel(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})Z$/.exec(name)
  if (!match) return name
  return `${match[1]} ${match[2]}:${match[3]}:${match[4]} UTC`
}

/**
 * The player's game files — engine settings, hotkeys, widget state — where
 * they can be copied from, and the copies taken before each launch.
 *
 * Every copy in either direction snapshots what is there first, so a wrong
 * click is one more entry in the list rather than a loss.
 */
export function PlayerFiles() {
  // A listing that fails is said, not thrown: the rest of the page still works.
  const [view, { refetch }] = createResource(async () => {
    try {
      return await api.playerFiles()
    } catch (error) {
      pushNotice('error', describeError(error))
      return null
    }
  })
  const [chosen, setChosen] = createSignal('')
  const [busy, setBusy] = createSignal(false)

  const snapshot = () => chosen() || view()?.snapshots[0] || ''

  async function copy(from: string, what: string) {
    setBusy(true)
    try {
      const count = await api.importPlayerFiles(from)
      pushNotice('info', `Copied ${count} files from ${what}.`)
      await refetch()
    } catch (error) {
      pushNotice('error', describeError(error))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class='player-files'>
      <p class='muted'>
        Engine settings, hotkeys and widget state live in the data directory. A
        copy of them is taken before every launch, under modlobby-backups, and
        the last ten are kept.
      </p>
      <For each={view()?.sources ?? []}>
        {(dir) => (
          <button
            type='button'
            disabled={busy()}
            onClick={() => void copy(dir, dir)}
          >
            Copy game settings from {dir}
          </button>
        )}
      </For>
      <Show when={view()?.snapshots.length}>
        <label class='row'>
          Copy from before a launch
          <Select
            value={snapshot()}
            onChange={(e) => setChosen(e.currentTarget.value)}
          >
            <For each={view()?.snapshots ?? []}>
              {(dir) => <option value={dir}>{snapshotLabel(dir)}</option>}
            </For>
          </Select>
          <button
            type='button'
            disabled={busy() || !snapshot()}
            onClick={() => void copy(snapshot(), snapshotLabel(snapshot()))}
          >
            Put back
          </button>
        </label>
      </Show>
    </div>
  )
}
