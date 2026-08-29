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
import { connectChannel } from './ipc/channel'
import { api, describeError } from './ipc/client'
import type { Settings } from './ipc/bindings/Settings'
import { chat, pushNotice } from './store/chat'
import { lobby } from './store/lobby'
import { applySettings } from './store/settings'
import { BattleList } from './views/BattleList'
import { Login } from './views/Login'
import { Room } from './views/Room'
import { SettingsView } from './views/Settings'

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
  onMount(async () => {
    const unlisten = await listen<SettingsEvent>('settings', (event) => {
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
    onCleanup(unlisten)
  })

  return (
    <div class='shell'>
      <nav class='nav'>
        <span class='brand'>modlobby</span>
        <Show when={lobby.phase === 'ready'}>
          <A href='/battles'>Battles</A>
          <Show when={lobby.myBattle}>
            <A href='/room'>Room</A>
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
      <Route path='/room' component={Room} />
      <Route path='/settings' component={SettingsView} />
    </HashRouter>
  )
}
