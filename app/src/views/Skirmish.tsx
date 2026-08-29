import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { mapNames } from '../lib/maps'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/**
 * A game against AI, with no server involved.
 *
 * The engine takes a start script where it would take a `spring://` URL, so
 * this needs nothing from the lobby at all — it works logged out, and it works
 * when the server is down.
 */
export function Skirmish() {
  const [options] = createResource(async () => {
    try {
      return await api.skirmishOptions()
    } catch (error) {
      pushNotice('warning', describeError(error))
      return null
    }
  })

  /**
   * Installed maps are archive file names — lowercased and underscored — and
   * the engine wants the map's real spring name. Nothing on disk records the
   * capitalisation, so it comes from BAR's published map index, the same one
   * the minimaps come from. Offline, the file name is offered as-is and says so.
   */
  const [names] = createResource(mapNames)

  const maps = createMemo(() => {
    const files = options()?.maps ?? []
    const index = names() ?? {}
    return files.map((file) => ({
      file,
      spring: index[file] ?? null,
      label: index[file] ?? file,
    }))
  })

  const [game, setGame] = createSignal('')
  const [map, setMap] = createSignal('')
  const [engine, setEngine] = createSignal('')
  const [ai, setAi] = createSignal('')
  const [count, setCount] = createSignal(1)
  const [search, setSearch] = createSignal('')
  const [busy, setBusy] = createSignal(false)

  // Whatever is newest, so the form is usable without touching it.
  const chosenGame = () => game() || (options()?.games[0] ?? '')
  const chosenEngine = () => engine() || (options()?.engines[0] ?? '')
  const chosenAi = () => ai() || (options()?.ais[0] ?? '')

  const shown = createMemo(() => {
    const needle = search().trim().toLowerCase()
    const all = maps()
    const matching = needle
      ? all.filter((entry) => entry.label.toLowerCase().includes(needle))
      : all
    return matching.slice(0, 400)
  })

  const chosenMap = createMemo(() =>
    maps().find((entry) => entry.file === map()),
  )
  const ready = () =>
    Boolean(chosenGame() && chosenEngine() && chosenMap()) &&
    lobby.engine.state !== 'running'

  async function start() {
    const entry = chosenMap()
    if (!entry) return
    setBusy(true)
    try {
      await api.startSkirmish(
        chosenGame(),
        // The spring name when we know it; the file name is the best guess we
        // have when the map index could not be reached.
        entry.spring ?? entry.file,
        chosenEngine(),
        Array.from({ length: count() }, () => chosenAi()),
      )
    } catch (error) {
      pushNotice('warning', describeError(error))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section class='skirmish'>
      <header class='chat-head'>
        <h1>Skirmish</h1>
        <span class='muted'>a game against AI, no server needed</span>
      </header>

      <Show
        when={options()}
        fallback={<p class='muted empty-list'>Looking at what is installed…</p>}
      >
        {(installed) => (
          <div class='skirmish-body'>
            <div class='skirmish-form'>
              <label>
                Game
                <select
                  value={chosenGame()}
                  onChange={(e) => setGame(e.currentTarget.value)}
                >
                  <For each={installed().games}>
                    {(name) => <option value={name}>{name}</option>}
                  </For>
                </select>
              </label>

              <label>
                Engine
                <select
                  value={chosenEngine()}
                  onChange={(e) => setEngine(e.currentTarget.value)}
                >
                  <For each={installed().engines}>
                    {(name) => <option value={name}>{name}</option>}
                  </For>
                </select>
              </label>

              <label>
                Opponent
                <select
                  value={chosenAi()}
                  onChange={(e) => setAi(e.currentTarget.value)}
                >
                  <For each={installed().ais}>
                    {(name) => <option value={name}>{name}</option>}
                  </For>
                </select>
              </label>

              <label>
                How many
                <input
                  type='number'
                  min='1'
                  max='15'
                  value={count()}
                  onChange={(e) =>
                    setCount(
                      Math.min(15, Math.max(1, Number(e.currentTarget.value))),
                    )
                  }
                />
              </label>

              <Show when={chosenMap() && !chosenMap()!.spring}>
                <p class='muted'>
                  The map index could not be reached, so this map is sent by its
                  file name. If the engine cannot find it, try again online.
                </p>
              </Show>

              <button
                class='primary'
                disabled={!ready() || busy()}
                onClick={start}
              >
                <Show when={lobby.engine.state === 'running'} fallback='Start'>
                  Engine running
                </Show>
              </button>
            </div>

            <div class='skirmish-maps'>
              <input
                class='search'
                placeholder={`Search ${maps().length} installed maps`}
                value={search()}
                onInput={(e) => setSearch(e.currentTarget.value)}
              />
              <div class='map-list'>
                <For
                  each={shown()}
                  fallback={<p class='muted setup-empty'>No map matches.</p>}
                >
                  {(entry) => (
                    <button
                      class='room-tab'
                      classList={{ on: map() === entry.file }}
                      onClick={() => setMap(entry.file)}
                    >
                      <span class='room-name'>{entry.label}</span>
                    </button>
                  )}
                </For>
              </div>
            </div>
          </div>
        )}
      </Show>
    </section>
  )
}
