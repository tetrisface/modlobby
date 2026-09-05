import { Show, createSignal } from 'solid-js'
import { ActionCell, CellButton } from '../components/ActionCell'
import { Ask } from '../components/Ask'

/**
 * The room's name, with a pen beside it in the tweak slots' look.
 *
 * Renaming is what Chobby does when its title button is pressed: `!rename`,
 * a SPADS command sent as chat. The pen is drawn only when `canRename` says
 * the command would be taken rather than refused.
 */
export function RoomTitle(props: {
  title: string
  canRename: boolean
  onRename: (name: string) => void
}) {
  const [asking, setAsking] = createSignal(false)

  return (
    <div class='room-title'>
      <h1 title={props.title}>{props.title}</h1>
      <Show when={props.canRename}>
        <ActionCell>
          <CellButton
            icon='act-pen'
            title='Rename room'
            onClick={() => setAsking(true)}
          />
        </ActionCell>
      </Show>
      <Show when={asking()}>
        <Ask
          title='Rename room'
          initial={props.title}
          confirm='Rename'
          onCancel={() => setAsking(false)}
          onAnswer={(name) => {
            setAsking(false)
            if (name !== props.title) props.onRename(name)
          }}
        />
      </Show>
    </div>
  )
}
