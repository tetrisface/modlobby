import { useNavigate } from '@solidjs/router'
import { createVirtualizer } from '@tanstack/solid-virtual'
import { For, Show, createMemo, createSignal, onMount } from 'solid-js'
import type { BattleList as Filters } from '../ipc/bindings/BattleList'
import type { BattleSort } from '../ipc/bindings/BattleSort'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { ModeFilter } from '../ipc/bindings/ModeFilter'
import { api, describeError } from '../ipc/client'
import { MODES, SORTS, arrange, type Row } from '../lib/battles'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { applySettings, settings } from '../store/settings'

const ROW_HEIGHT = 44

const DEFAULTS: Filters = {
  showPassworded: true,
  showLocked: true,
  showEmpty: true,
  showRunning: true,
  friendsOnly: false,
  mode: 'all',
  sort: 'relevance',
  sortDescending: false,
}

export function BattleList() {
  const navigate = useNavigate()
  const [search, setSearch] = createSignal('')
  let scrollRef: HTMLDivElement | undefined

  const filters = (): Filters => settings()?.battleList ?? DEFAULTS

  /** Filters live in settings so the list looks the same next launch. */
  async function update(patch: Partial<Filters>) {
    const current = settings()
    if (!current) return
    try {
      applySettings(
        await api.updateSettings({
          ...current,
          battleList: { ...filters(), ...patch },
        }),
      )
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  const friends = createMemo(() => new Set(lobby.friends.friends))

  const all = createMemo<Row[]>(() => {
    const known = friends()
    return Object.values(lobby.battles).map((battle) => ({
      battle,
      running: lobby.users[battle.founder]?.status.inGame ?? false,
      hasFriend: battle.members.some((name) => known.has(name)),
    }))
  })
  const rows = createMemo(() => arrange(all(), filters(), search()))

  const virtualizer = createVirtualizer({
    get count() {
      return rows().length
    },
    getScrollElement: () => scrollRef ?? null,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  })

  async function join(battle: BattleView) {
    const password = battle.passworded ? window.prompt('Room password') : null
    if (battle.passworded && password === null) return
    try {
      await api.joinBattle(battle.id, password)
      navigate('/room')
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  /** Clicking the sort you are already on flips it, as a table header would. */
  function sortBy(key: BattleSort) {
    if (key === filters().sort && key !== 'relevance')
      return update({ sortDescending: !filters().sortDescending })
    return update({ sort: key, sortDescending: key === 'players' })
  }

  const hidden = createMemo(() => all().length - rows().length)

  /**
   * The room we were in when the app last stopped without leaving it. Offered
   * only while it is still open — a room that closed while we were away is
   * nothing to go back to.
   */
  const [remembered, setRemembered] = createSignal<number | null>(null)
  onMount(async () => {
    try {
      setRemembered(await api.rememberedBattle())
    } catch {
      // Never having an offer is a fine outcome; it is not worth a notice.
    }
  })
  const rejoinable = createMemo(() => {
    const id = remembered()
    if (id === null || lobby.myBattle) return undefined
    return lobby.battles[id]
  })

  async function forget() {
    setRemembered(null)
    try {
      await api.forgetBattle()
    } catch {
      // Dismissed either way; the file is not worth a warning.
    }
  }

  return (
    <section class='battles'>
      <header class='toolbar'>
        <input
          class='search'
          placeholder='Search title, map, host, game'
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
        />

        {/* Each says what it lets through, so an unlit one is a room type
            you have switched off. No label needed above them. */}
        <div class='filter-group' role='group' aria-label='Show rooms that are'>
          <Include
            label='Passworded'
            on={filters().showPassworded}
            onClick={() =>
              update({ showPassworded: !filters().showPassworded })
            }
          />
          <Include
            label='Locked'
            on={filters().showLocked}
            onClick={() => update({ showLocked: !filters().showLocked })}
          />
          <Include
            label='Running'
            on={filters().showRunning}
            onClick={() => update({ showRunning: !filters().showRunning })}
          />
          <Include
            label='Empty'
            on={filters().showEmpty}
            onClick={() => update({ showEmpty: !filters().showEmpty })}
          />
        </div>

        {/* Shown while it is on even with no friends loaded: hiding the
            control that is emptying the list would strand the reader. */}
        <Show when={filters().friendsOnly || lobby.friends.friends.length > 0}>
          <div class='filter-group' role='group' aria-label='Friends'>
            <Choice
              label='Friends only'
              on={filters().friendsOnly}
              onClick={() => update({ friendsOnly: !filters().friendsOnly })}
            />
          </div>
        </Show>

        <div class='filter-group' role='group' aria-label='Mode'>
          <For each={MODES}>
            {(mode) => (
              <Choice
                label={mode.label}
                on={filters().mode === mode.key}
                onClick={() => update({ mode: mode.key as ModeFilter })}
              />
            )}
          </For>
        </div>

        <div class='filter-group' role='group' aria-label='Sort'>
          <span class='filter-label'>Sort</span>
          <For each={SORTS}>
            {(sort) => (
              <Choice
                label={
                  filters().sort === sort.key && sort.key !== 'relevance'
                    ? `${sort.label} ${filters().sortDescending ? '↓' : '↑'}`
                    : sort.label
                }
                on={filters().sort === sort.key}
                onClick={() => sortBy(sort.key)}
              />
            )}
          </For>
        </div>

        <span class='spacer' />
        <span class='muted count'>
          {rows().length} rooms
          <Show when={hidden() > 0}> · {hidden()} hidden</Show> ·{' '}
          {Object.keys(lobby.users).length} users
        </span>
      </header>

      <Show when={rejoinable()}>
        {(battle) => (
          <div class='rejoin'>
            <span class='rejoin-what'>
              You were in <b>{battle().title}</b> when modlobby last closed.
            </span>
            <button
              class='primary'
              onClick={() => {
                // Read the room before clearing: clearing unmounts this
                // block, and with it the accessor the value came from.
                const room = battle()
                setRemembered(null)
                void join(room)
              }}
            >
              Rejoin
            </button>
            <button onClick={forget}>Not now</button>
          </div>
        )}
      </Show>

      <div class='list' ref={scrollRef}>
        <Show
          when={rows().length > 0}
          fallback={
            <p class='muted empty-list'>
              <Show when={all().length > 0} fallback='No rooms open right now.'>
                Nothing matches. {hidden()} rooms are hidden by the filters
                above.
              </Show>
            </p>
          }
        >
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              position: 'relative',
            }}
          >
            <For each={virtualizer.getVirtualItems()}>
              {(item) => {
                const row = () => rows()[item.index]
                return (
                  <Show when={row()}>
                    {(r) => (
                      <div
                        class='battle-row'
                        classList={{
                          running: r().running,
                          locked: r().battle.locked,
                        }}
                        style={{
                          position: 'absolute',
                          top: `${item.start}px`,
                          height: `${ROW_HEIGHT}px`,
                          width: '100%',
                        }}
                        onDblClick={() => join(r().battle)}
                      >
                        <span class='col-players'>
                          {r().battle.playerCount}/{r().battle.maxPlayers}
                          <small> +{r().battle.spectatorCount}</small>
                        </span>
                        <span class='col-layout'>
                          {r().battle.layout
                            ? `${r().battle.layout?.teams}x${r().battle.layout?.teamSize}`
                            : ''}
                        </span>
                        <span class='col-title' title={r().battle.title}>
                          {r().battle.title}
                        </span>
                        <span class='col-map'>{r().battle.mapName}</span>
                        <span class='col-flags'>
                          {r().running ? '▶ ' : ''}
                          {r().battle.locked ? '🔒 ' : ''}
                          {r().battle.passworded ? '🔑' : ''}
                        </span>
                        <button onClick={() => join(r().battle)}>
                          Spectate
                        </button>
                      </div>
                    )}
                  </Show>
                )
              }}
            </For>
          </div>
        </Show>
      </div>
    </section>
  )
}

/**
 * A room type the list lets through. On is the resting state, so the eye is
 * drawn to what you have switched off rather than to four lit chips.
 */
function Include(props: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      class='chip-include'
      classList={{ off: !props.on }}
      aria-pressed={props.on}
      title={
        props.on
          ? `Hide ${props.label.toLowerCase()} rooms`
          : `Show ${props.label.toLowerCase()} rooms`
      }
      onClick={props.onClick}
    >
      {props.label}
    </button>
  )
}

/** One of a set, where exactly one is active. */
function Choice(props: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      class='chip-choice'
      classList={{ on: props.on }}
      aria-pressed={props.on}
      onClick={props.onClick}
    >
      {props.label}
    </button>
  )
}
