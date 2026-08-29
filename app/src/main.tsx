/* @refresh reload */
import { render } from 'solid-js/web'
import { App } from './App'
import { captureConsole } from './ipc/logging'
// Bundled, not fetched: a lobby has to look right before the network is up,
// and the webview's CSP admits no font host. Latin subset, used weights only.
import '@fontsource/chakra-petch/latin-600.css'
import '@fontsource/chakra-petch/latin-700.css'
import '@fontsource/ibm-plex-sans/latin-400.css'
import '@fontsource/ibm-plex-sans/latin-500.css'
import '@fontsource/ibm-plex-sans/latin-600.css'
import '@fontsource/ibm-plex-mono/latin-400.css'
import '@fontsource/ibm-plex-mono/latin-500.css'
import 'flag-icons/css/flag-icons.min.css'
import './styles.css'

captureConsole()

const root = document.getElementById('root')
if (!root) throw new Error('missing #root')
render(() => <App />, root)
