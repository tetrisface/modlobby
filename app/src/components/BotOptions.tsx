import { Show, createMemo, createResource } from 'solid-js'
import type { BotView } from '../ipc/bindings/BotView'
import { api, describeError } from '../ipc/client'
import { rowsOf } from '../lib/setup'
import { pushNotice } from '../store/chat'
import { Rows } from '../views/Setup'
import { useRoom } from '../views/room/model'

/**
 * What an AI can be told about itself.
 *
 * An engine AI declares its own options in `AIOptions.lua`, which is the same
 * `local options = { … }` table BAR's `modoptions.lua` is — so the same parser
 * reads it and the same rows draw it, tabs and groups and all. What is set
 * here rides into the start script as that AI's `[options]` block, which is
 * where its difficulty lives.
 *
 * A game's Lua AIs — Scavengers, Raptors — have no such file. They are
 * configured by the game's own modoptions, in the pane on the right, and this
 * says so rather than showing an empty list.
 */
export function BotOptions(props: { bot: BotView; onClose: () => void }) {
  const room = useRoom()

  const [catalogue] = createResource(
    () => [room.battle()?.engineVersion ?? '', props.bot.ai] as const,
    ([engine, ai]) =>
      engine === '' ? [] : api.aiOptions(engine, ai).catch(() => []),
  )

  /** What this AI has been told, out of the room's own record of it. */
  const values = createMemo(() => {
    const held = room
      .battle()
      ?.bots.find((bot) => bot.name === props.bot.name)?.options
    return held ?? {}
  })

  const rows = createMemo(() =>
    rowsOf({ name: '', options: catalogue() ?? [] }, values()),
  )

  async function set(key: string, value: string) {
    try {
      await room.io.setBotOption(props.bot.name, key, value)
    } catch (error) {
      pushNotice('warning', `${props.bot.name}: ${describeError(error)}`)
    }
  }

  return (
    <div class='sheet' onMouseDown={props.onClose}>
      <div
        class='sheet-card bot-options'
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class='ed-head'>
          <h2>
            {props.bot.name} <span class='muted'>{props.bot.ai}</span>
          </h2>
          <span class='spacer' />
          <button type='button' onClick={props.onClose}>
            Close
          </button>
        </header>

        <Show
          when={!catalogue.loading}
          fallback={<p class='muted setup-empty'>Reading what it takes…</p>}
        >
          <Show
            when={rows().length > 0}
            fallback={
              <p class='muted setup-note'>
                {props.bot.ai} declares no options of its own. A game's own AIs
                — Scavengers, Raptors — are set up in the room's settings
                instead.
              </p>
            }
          >
            <div class='setup-detail'>
              <Rows rows={rows()} editable onEdit={() => {}} set={set} />
            </div>
          </Show>
        </Show>
      </div>
    </div>
  )
}
