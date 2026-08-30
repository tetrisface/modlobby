import { A, HashRouter, Navigate, Route } from '@solidjs/router'
import { listen } from '@tauri-apps/api/event'
import {
  For,
  Show,
  createEffect,
  onCleanup,
  onMount,
  type ParentProps,
} from 'solid-js'
import { IconSprite } from './components/icons'
import { PlayerMenu } from './components/PlayerMenu'
import { connectChannel } from './ipc/channel'
import { api, describeError } from './ipc/client'
import type { Settings } from './ipc/bindings/Settings'
import { chat, pushNotice } from './store/chat'
import { lobby } from './store/lobby'
import { applySettings } from './store/settings'
import { BattleList } from './views/BattleList'
import { Chat } from './views/Chat'
import { Login } from './views/Login'
import { Replays } from './views/Replays'
import { Room } from './views/Room'
import { SettingsView } from './views/Settings'
import { Skirmish } from './views/Skirmish'
import { Tweaks } from './views/Tweaks'

type SettingsEvent = { changed: Settings } | { invalid: string }

function Layout(props: ParentProps) {
  onMount(async () => {
    try {
      applySettings(await api.getSettings())
      await connectChannel()
    } catch (error) {
      pushNotice('error', describeError(error))
    }
  })
  onMount(() => {
    // `listen` resolves after a round trip. Registering the cleanup on the
    // promise keeps it inside this component's owner — awaiting first would
    // leave the owner behind and the listener would never be removed.
    const pending = listen<SettingsEvent>('settings', (event) => {
      if ('changed' in event.payload) {
        applySettings(event.payload.changed)
        pushNotice('info', 'settings reloaded')
      } else {
        pushNotice(
          'warning',
          `settings file not applied: ${event.payload.invalid}`,
        )
      }
    })
    onCleanup(() => void pending.then((unlisten) => unlisten()))
  })

  return (
    <div class='shell'>
      <IconSprite />
      <nav class='nav'>
        <span class='brand'>modlobby</span>
        <A href='/skirmish'>Skirmish</A>
        <Show when={lobby.phase === 'ready'}>
          <A href='/battles'>Battles</A>
          <A href='/chat'>Chat</A>
          <A href='/replays'>Replays</A>
          <Show when={lobby.myBattle}>
            <A href='/room'>Room</A>
            <A href='/room/tweaks'>Tweaks</A>
          </Show>
        </Show>
        <A href='/settings'>Settings</A>
        <span class='spacer' />
        <Show
          when={lobby.me}
          fallback={<span class='muted'>not logged in</span>}
        >
          <span>{lobby.me}</span>
          <button onClick={() => api.logout()}>Log out</button>
        </Show>
      </nav>
      <main class='main'>{props.children}</main>
      <Notices />
      <PlayerMenu />
    </div>
  )
}

function Notices() {
  return (
    <div class='notices'>
      <For each={chat.notices.slice(-3)}>
        {(notice) => <div class={`notice ${notice.level}`}>{notice.text}</div>}
      </For>
    </div>
  )
}

function Home() {
  createEffect(() => lobby.phase)
  return (
    <Show when={lobby.phase === 'ready'} fallback={<Login />}>
      <Navigate href='/battles' />
    </Show>
  )
}

export function App() {
  return (
    <HashRouter root={Layout}>
      <Route path='/' component={Home} />
      <Route path='/login' component={Login} />
      <Route path='/battles' component={BattleList} />
      <Route path='/chat' component={Chat} />
      <Route path='/replays' component={Replays} />
      <Route path='/skirmish' component={Skirmish} />
      <Route path='/room' component={Room} />
      <Route path='/room/tweaks' component={Tweaks} />
      <Route path='/settings' component={SettingsView} />
    </HashRouter>
  )
}
