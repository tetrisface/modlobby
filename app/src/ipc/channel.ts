import { Channel } from '@tauri-apps/api/core'
import type { UiMessage } from './bindings/UiMessage'
import { applyMessage } from '../store/apply'
import { api } from './client'

/** Opens the one stream the runtime pushes into; a snapshot arrives first. */
export async function connectChannel(): Promise<void> {
  const channel = new Channel<UiMessage>()
  channel.onmessage = applyMessage
  await api.subscribe(channel)
}
