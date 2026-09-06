import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { PlayerFilesView } from '../ipc/bindings/PlayerFilesView'
import { PlayerFiles, snapshotLabel } from './PlayerFiles'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

afterEach(() => {
  cleanup()
  invoke.mockReset()
})

const launcher = 'C:\\u\\Programs\\Beyond-All-Reason\\data'
const newest = 'C:\\u\\modlobby\\data\\modlobby-backups\\2026-09-06T00-48-12Z'
const older = 'C:\\u\\modlobby\\data\\modlobby-backups\\2026-09-05T11-30-21Z'

const view: PlayerFilesView = {
  write: 'C:\\u\\modlobby\\data',
  sources: [launcher],
  snapshots: [newest, older],
}

function answering(files: PlayerFilesView) {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'player_files') return files
    if (command === 'import_player_files') return 3
    throw new Error(`unexpected ${command}`)
  })
}

describe('snapshotLabel', () => {
  test('a snapshot directory reads as the time it was taken', () => {
    expect(snapshotLabel(newest)).toBe('2026-09-06 00:48:12 UTC')
    expect(snapshotLabel('/x/modlobby-backups/2026-09-05T11-30-21Z')).toBe(
      '2026-09-05 11:30:21 UTC',
    )
  })

  test('anything else is shown as it is', () => {
    expect(snapshotLabel('C:\\somewhere\\else')).toBe('else')
  })
})

describe('PlayerFiles', () => {
  test('each install with settings worth copying gets a button', async () => {
    answering(view)
    const { findByText } = render(() => <PlayerFiles />)
    const button = await findByText(`Copy game settings from ${launcher}`)
    fireEvent.click(button)
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('import_player_files', {
        from: launcher,
      }),
    )
  })

  test('the newest snapshot is offered first and put back on request', async () => {
    answering(view)
    const { findByText, getByRole } = render(() => <PlayerFiles />)
    const button = await findByText('Put back')
    const select = getByRole('combobox') as HTMLSelectElement
    expect(select.options.item(0)?.textContent).toBe('2026-09-06 00:48:12 UTC')
    expect(select.value).toBe(newest)

    fireEvent.change(select, { target: { value: older } })
    fireEvent.click(button)
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('import_player_files', {
        from: older,
      }),
    )
  })

  test('nothing to copy from and nothing kept shows only the explanation', async () => {
    answering({ ...view, sources: [], snapshots: [] })
    const { findByText, queryByText, queryByRole } = render(() => (
      <PlayerFiles />
    ))
    await findByText(/Engine settings, hotkeys and widget state/)
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('player_files'))
    expect(queryByText(/Copy game settings from/)).toBeNull()
    expect(queryByRole('combobox')).toBeNull()
  })
})
