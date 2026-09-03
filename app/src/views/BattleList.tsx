import { useNavigate } from '@solidjs/router'
import { createVirtualizer } from '@tanstack/solid-virtual'
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js'
import type { BattleList as Filters } from '../ipc/bindings/BattleList'
import type { BattleSort } from '../ipc/bindings/BattleSort'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { ModeFilter } from '../ipc/bindings/ModeFilter'
import { RankIcon } from '../components/icons'
import { Thinking } from '../components/Thinking'
import { api, describeError } from '../ipc/client'
import { elapsed } from '../lib/running'
import {
  asking as askingAbout,
  askAboutGame,
  noteRunning,
  running as runningGames,
} from '../store/running'
import { MODES, SORTS, arrange, stabilize, type Row } from '../lib/battles'
import { mapThumb } from '../lib/maps'
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
  const sorted = createMemo(() => arrange(all(), filters(), search()))

  /**
   * The order held still while the pointer is inside the list, as battle ids.
   *
   * On a busy evening the sort keys change every few seconds, and the room
   * you are reaching for jumps away as you reach. So entering the list pins
   * the order and leaving it lets the fresh sort through; rows keep updating
   * their contents in place the whole time. Changing what is asked for —
   * sort, filters, search — releases the pin too, because then the reader
   * asked for the movement.
   */
  const [held, setHeld] = createSignal<number[] | null>(null)
  createEffect(() => {
    filters()
    search()
    setHeld(null)
  })

  const rows = createMemo(() => {
    const pinned = held()
    return pinned === null ? sorted() : stabilize(sorted(), pinned)
  })

  // How long each running game has been going. The store holds it, because a
  // host can also tell us outright — see `store/running`.
  const [now, setNow] = createSignal(Date.now())
  let looked = false

  createEffect(() => {
    const going = new Set(
      all()
        .filter((row) => row.running)
        .map((row) => row.battle.id),
    )
    noteRunning(going, looked)
    // Anything running at the first look was already running before it.
    if (lobby.phase === 'ready') looked = true
  })

  // A minute is the resolution the label has, so it is the rate it needs.
  const tick = setInterval(() => setNow(Date.now()), 30_000)
  onCleanup(() => clearInterval(tick))

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

  /**
   * What clicking a room does, as Chobby asks it: remember what you did last
   * time, or always one or the other (`gui_settings_window.lua:906`).
   */
  const posture = (): 'player' | 'spectator' => {
    const play = settings()?.play
    if (!play) return 'spectator'
    if (play.joinAs === 'remember')
      return play.lastWasPlayer ? 'player' : 'spectator'
    return play.joinAs
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

  /**
   * A room of your own without joining one first: the same empty autohost
   * the Seat bar takes over from inside a room. The runtime answers with the
   * room it picked as soon as it has asked to join, so the walk to the room
   * waits for the host to let us in — going there earlier would only be
   * bounced straight back here.
   */
  const [hosting, setHosting] = createSignal<number | null>(null)
  const [hostBusy, setHostBusy] = createSignal(false)
  createEffect(() => {
    const wanted = hosting()
    const mine = lobby.myBattle?.id
    if (wanted === null || mine === undefined) return
    // Whatever room we ended up in settles the wait; a refused host is a
    // notice from the runtime, and the next room clicked is its own choice.
    setHosting(null)
    if (mine === wanted) navigate('/room')
  })

  async function host() {
    setHostBusy(true)
    try {
      setHosting(await api.hostPublic())
    } catch (error) {
      pushNotice('warning', `host a room: ${describeError(error)}`)
    } finally {
      setHostBusy(false)
    }
  }

  const hidden = createMemo(() => all().length - rows().length)

  /**
   * The room we were in when the app last stopped without leaving it. Offered
   * only while it is still open — a room that closed while we were away is
   * nothing to go back to.
   */
  const [remembered, setRemembered] = createSignal<number | null>(null)
  const [peek, setPeek] = createSignal<Peek | null>(null)
  /**
   * Where the pointer is, kept apart from `peek` on purpose: rolling it into
   * that object would make the card's name list re-sort on every mouse move,
   * for a change that only moves a box sideways.
   */
  const [pointerX, setPointerX] = createSignal(0)
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
        <button
          class='primary'
          disabled={hostBusy()}
          title='Take over an empty autohost near you and become its boss'
          onClick={() => void host()}
        >
          Host battle
        </button>
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

      <div
        class='list'
        ref={scrollRef}
        onPointerEnter={() => setHeld(rows().map((row) => row.battle.id))}
        onPointerLeave={() => setHeld(null)}
      >
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
            onMouseMove={(event) => setPointerX(event.clientX)}
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
                        role='button'
                        tabindex={0}
                        onClick={() => join(r().battle)}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault()
                            join(r().battle)
                          }
                        }}
                        onMouseEnter={(event) => {
                          setPointerX(event.clientX)
                          setPeek({
                            id: r().battle.id,
                            top: event.currentTarget.getBoundingClientRect()
                              .top,
                          })
                          // Resting on a running room is what asks its host
                          // how long the game has been going.
                          if (r().running)
                            askAboutGame(r().battle.id, r().battle.founder)
                        }}
                      >
                        <MapThumb
                          src={mapThumb(
                            r().battle.mapName,
                            THUMB.width,
                            THUMB.height,
                          )}
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
                        <span class='col-title'>{r().battle.title}</span>
                        <span class='col-map'>{r().battle.mapName}</span>
                        <span class='col-flags'>
                          {/* The host is being asked how long its game has
                              been going; without this the answer arrives out
                              of nowhere a second later. */}
                          <Show when={askingAbout() === r().battle.id}>
                            <span class='running-for'>
                              <Thinking title='asking the host how long this game has been running' />
                            </span>
                          </Show>
                          <Show
                            when={r().running && runningGames()[r().battle.id]}
                          >
                            {(going) => (
                              <span
                                class='running-for'
                                title='How long this game has been running. A + means it was already going when you logged in, so this is the least it can be.'
                              >
                                {elapsed(going(), now())}
                              </span>
                            )}
                          </Show>
                          {r().running ? '▶ ' : ''}
                          {r().battle.locked ? '🔒 ' : ''}
                          {r().battle.passworded ? '🔑' : ''}
                        </span>
                      </div>
                    )}
                  </Show>
                )
              }}
            </For>
          </div>
        </Show>

        <Occupants peek={peek()} now={now()} x={pointerX()} />

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
type Peek = { id: number; top: number }

/** Kept in step with `.occupants` in the stylesheet. */
const CARD_WIDTH = 300
/** Chobby's own tooltip offset (`gui_tooltip.lua:1262`). */
const CARD_GAP = 20

/**
 * Where the card goes for a pointer at `x`.
 *
 * Beside the pointer, as Chobby's tooltip does, rather than pinned to an edge:
 * a row spans the whole window, so an edge-pinned card always covers half of
 * what is being read, and flipping between the two edges moves it further than
 * the eye wants to travel.
 *
 * One deliberate difference from Chobby, which clamps to the right edge when
 * the tooltip would overflow (`gui_tooltip.lua:1276`): clamping leaves the card
 * sitting under the cursor near that edge, which is the covering-things
 * complaint this set out to fix. It opens to the left of the pointer instead.
 */
export function cardLeft(x: number, viewport: number): number {
  const right = x + CARD_GAP
  if (right + CARD_WIDTH <= viewport - CARD_GAP) return right
  return Math.max(CARD_GAP, x - CARD_GAP - CARD_WIDTH)
}

function Occupants(props: { peek: Peek | null; now: number; x: number }) {
  const battle = () =>
    props.peek === null ? undefined : lobby.battles[props.peek.id]

  /**
   * How long this room's game has been going, when it is going.
   *
   * Worth having here as well as in the row, because the answer usually
   * arrives after you have moved on: the host is asked at most once every
   * 400ms and only about the room the pointer settled on, so the reply lands
   * a moment later. Coming back to the row is how you read it, and the `+`
   * falling away is how you know the host answered rather than that we are
   * still counting from when we first looked.
   */
  const running = () => {
    const id = props.peek?.id
    if (id === undefined) return undefined
    const held = runningGames()[id]
    if (
      !held ||
      !(lobby.users[battle()?.founder ?? '']?.status.inGame ?? false)
    )
      return undefined
    return elapsed(held, props.now)
  }

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
          left: `${cardLeft(props.x, window.innerWidth)}px`,
        }}
      >
        <div class='occupants-head'>
          {people().length} here
          <Show when={running()}>
            {(going) => (
              <span
                class='occupants-running'
                title='How long this game has been running. A + means the host has not answered yet, so this is only as long as we have been watching.'
              >
                {' · running '}
                {going()}
              </span>
            )}
          </Show>
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

/**
 * A room's map, when the index knows it; an empty frame when it does not.
 *
 * The picture is the published one, drawn into a 52px column: the same URL
 * the official lobby fetches, so it comes out of a shared CDN cache and stays
 * in the webview's for good. Lazy, so a long list does not ask for every
 * picture at once.
 */
/**
 * The inside of `.col-thumb` (`styles.css`), which the picture is made to fill
 * exactly. Change both together.
 */
const THUMB = { width: 50, height: 32 }

function MapThumb(props: { src: string | null; alt: string }) {
  const [broken, setBroken] = createSignal(false)

  // The rows are virtualised, so this component stays mounted at its place on
  // screen while different rooms scroll through it. Without this, one map whose
  // picture failed to load would blank that row position for every room that
  // came after it — and the longer you scrolled, the more of the list went
  // empty. A new picture gets its own chance to fail.
  createEffect(() => {
    void props.src
    setBroken(false)
  })

  return (
    <span class='col-thumb'>
      <Show when={broken() ? undefined : props.src}>
        {(src) => (
          <img
            src={src()}
            alt={props.alt}
            loading='lazy'
            decoding='async'
            onError={() => setBroken(true)}
          />
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
