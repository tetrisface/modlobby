/* @refresh reload */
import { render } from 'solid-js/web'
import { App } from './App'
import { AfterUpdate } from './components/AfterUpdate'
import { boot } from './ipc/boot'
import { captureConsole, milestone, reportStart } from './ipc/logging'
import { setBuild } from './store/build'
import { applySettings } from './store/settings'
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
// Console hooks for staging room states by hand; see `dev.ts`.
if (import.meta.env.DEV) void import('./dev')

// In place before anything is drawn, so the first screen is drawn once: at
// the scale the settings ask for, with the version in its corner.
const start = boot()
if (start) {
	applySettings(start.settings)
	setBuild(start.version)
}

const root = document.getElementById('root')
if (!root) throw new Error('missing #root')
render(
	() => (
		<AfterUpdate>
			<App />
		</AfterUpdate>
	),
	root,
)
milestone('first render')

/**
 * The tweak editor is most of the app's script and no part of its first
 * screen, so it is fetched once the start is out of the way: early enough
 * that opening it never waits, late enough that the start never does.
 */
async function loadEditor(): Promise<void> {
	milestone('editor loading')
	await import('./views/tweaks/Workspace')
	milestone('editor loaded')
	reportStart()
}
// WebKit, which is the webview on macOS and Linux, has no idle callback: a
// short wait stands in for one there.
if ('requestIdleCallback' in window)
	requestIdleCallback(() => void loadEditor())
else setTimeout(() => void loadEditor(), 200)
