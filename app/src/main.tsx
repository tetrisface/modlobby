/* @refresh reload */
import { render } from 'solid-js/web'
import { App } from './App'
import { captureConsole } from './ipc/logging'
import './styles.css'

captureConsole()

const root = document.getElementById('root')
if (!root) throw new Error('missing #root')
render(() => <App />, root)
