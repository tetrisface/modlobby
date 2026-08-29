import { useNavigate } from '@solidjs/router'
import { createVirtualizer } from '@tanstack/solid-virtual'
import { For, Show, createMemo, createSignal } from 'solid-js'
import type { BattleView } from '../ipc/bindings/BattleView'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'

const ROW_HEIGHT = 44

export function BattleList() {
  const navigate = useNavigate()
  const [search, setSearch] = createSignal('')
  let scrollRef: HTMLDivElement | undefined

  const rows = createMemo(() => {
    const filters = settings()?.battleList
    const needle = search().trim().toLowerCase()
    const hostInGame = (b: BattleView) =>
      lobby.users[b.founder]?.status.inGame ?? false
    return Object.values(lobby.battles)
      .filter((b) => !(filters?.hidePassworded && b.passworded))
      .filter((b) => !(filters?.hideLocked && b.locked))
      .filter((b) => !(filters?.hideEmpty && b.playerCount === 0))
      .filter(
        (b) =>
          !needle ||
          b.title.toLowerCase().includes(needle) ||
          b.mapName.toLowerCase().includes(needle) ||
          b.founder.toLowerCase().includes(needle),
      )
      .sort((a, b) => b.playerCount - a.playerCount || a.id - b.id)
      .map((b) => ({ battle: b, running: hostInGame(b) }))
  })

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

  return (
    <section class='battles'>
      <header class='toolbar'>
        <input
          placeholder='Search title, map, host'
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
        />
        <span class='muted'>
          {rows().length} rooms · {Object.keys(lobby.users).length} users
        </span>
      </header>
      <div class='list' ref={scrollRef}>
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
                      <button onClick={() => join(r().battle)}>Spectate</button>
                    </div>
                  )}
                </Show>
              )
            }}
          </For>
        </div>
      </div>
    </section>
  )
}
