import { useNavigate } from '@solidjs/router'
import { createVirtualizer } from '@tanstack/solid-virtual'
import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  onMount,
} from 'solid-js'
import type { BattleList as Filters } from '../ipc/bindings/BattleList'
import type { BattleSort } from '../ipc/bindings/BattleSort'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { ModeFilter } from '../ipc/bindings/ModeFilter'
import { RankIcon } from '../components/icons'
import { api, describeError } from '../ipc/client'
import { MODES, SORTS, arrange, type Row } from '../lib/battles'
import { mapImages } from '../lib/maps'
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

  // One shared load for the whole list; a row without a picture just shows none.
  const [previews] = createResource(mapImages)

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

  /**
   * A passworded room asks first.
   *
   * Not with `window.prompt`: it stops the whole page until it is answered —
   * chat, the battle list and every timer with it — and WebKitGTK, which is
   * the webview everywhere that is not Windows, refuses it outright and hands
   * back `null`, so a passworded room could never be joined there at all.
   */
  async function join(battle: BattleView) {
    if (battle.passworded) return setAsking(battle)
    await enter(battle, null)
  }

  async function enter(battle: BattleView, password: string | null) {
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
  const [peek, setPeek] = createSignal<{ id: number; top: number } | null>(null)
  const [asking, setAsking] = createSignal<BattleView | null>(null)
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
            onMouseLeave={() => setPeek(null)}
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
                        onMouseEnter={(event) =>
                          setPeek({
                            id: r().battle.id,
                            top: event.currentTarget.getBoundingClientRect()
                              .top,
                          })
                        }
                      >
                        <MapThumb
                          src={previews()?.[r().battle.mapName]}
                          alt={r().battle.mapName}
                        />
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

        <Occupants peek={peek()} />

        <Show when={asking()}>
          {(battle) => (
            <PasswordDialog
              title={battle().title}
              onCancel={() => setAsking(null)}
              onEnter={(password) => {
                setAsking(null)
                void enter(battle(), password)
              }}
            />
          )}
        </Show>
      </div>
    </section>
  )
}

/**
 * Who is already in a room, shown while the pointer rests on it.
 *
 * The counts answer "is there room"; this answers "is it worth joining" —
 * which is the question you actually have, and the one that otherwise costs a
 * join and a leave to answer. Only names are knowable from outside a room:
 * the server sends battle status for the room you are in and no other, so
 * there is no way to say who here is playing and who is watching.
 */
function Occupants(props: { peek: { id: number; top: number } | null }) {
  const battle = () =>
    props.peek === null ? undefined : lobby.battles[props.peek.id]

  /** Friends first, then everyone else; the host bot is not a person. */
  const people = createMemo(() => {
    const room = battle()
    if (!room) return []
    const friends = new Set(lobby.friends.friends)
    return room.members
      .filter((name) => name !== room.founder)
      .sort((a, b) => {
        const known = Number(friends.has(b)) - Number(friends.has(a))
        return known || a.localeCompare(b)
      })
  })

  return (
    <Show when={props.peek && people().length > 0}>
      <aside
        class='occupants'
        style={{
          // Kept clear of the bottom edge; the pointer is on the row, so the
          // card can sit anywhere that does not cover it.
          top: `${Math.min(props.peek!.top, window.innerHeight - 320)}px`,
        }}
      >
        <div class='occupants-head'>
          {people().length} here
          <Show when={battle()?.founder}>
            {(host) => <span class='muted'> · hosted by {host()}</span>}
          </Show>
        </div>
        <div class='occupants-names'>
          <For each={people().slice(0, 28)}>
            {(name) => (
              <span
                classList={{
                  friend: lobby.friends.friends.includes(name),
                  me: name === lobby.me,
                }}
              >
                {/* The chevron is the one thing about a stranger everybody
                    already reads, and it is the difference between a room
                    worth joining and one you will be carried through. */}
                <Show when={lobby.users[name]}>
                  {(who) => <RankIcon status={who().status} />}
                </Show>
                {name}
              </span>
            )}
          </For>
        </div>
        <Show when={people().length > 28}>
          <div class='muted'>and {people().length - 28} more</div>
        </Show>
      </aside>
    </Show>
  )
}

/** Asks for a room's password without stopping the rest of the lobby. */
function PasswordDialog(props: {
  title: string
  onEnter: (password: string) => void
  onCancel: () => void
}) {
  const [password, setPassword] = createSignal('')
  let field: HTMLInputElement | undefined

  onMount(() => field?.focus())

  return (
    <div class='sheet' onMouseDown={props.onCancel}>
      <form
        class='sheet-card'
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault()
          props.onEnter(password())
        }}
      >
        <h2>{props.title}</h2>
        <p class='muted'>This room needs a password.</p>
        <input
          ref={field}
          type='password'
          value={password()}
          placeholder='Password'
          onInput={(event) => setPassword(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') props.onCancel()
          }}
        />
        <div class='sheet-actions'>
          <button type='button' onClick={props.onCancel}>
            Cancel
          </button>
          <button class='primary' type='submit'>
            Join
          </button>
        </div>
      </form>
    </div>
  )
}

/** A room's map, when the index knows it; an empty frame when it does not. */
function MapThumb(props: { src: string | undefined; alt: string }) {
  const [broken, setBroken] = createSignal(false)
  return (
    <span class='col-thumb'>
      <Show when={broken() ? undefined : props.src}>
        {(src) => (
          <img src={src()} alt={props.alt} onError={() => setBroken(true)} />
        )}
      </Show>
    </span>
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
