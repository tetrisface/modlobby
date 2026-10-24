import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, expect, test, vi } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import type { Settings } from '../ipc/bindings/Settings'
import { setSettingsSignal, settings } from '../store/settings'
import { ModsChip } from './ModsChip'
import { ModsConsent } from './ModsConsent'

vi.mock('@tauri-apps/api/core', async () => ({ invoke: vi.fn() }))

const SHA = '9108a17078f79d09925edc305ec83bc06c3a7cb3'

/** A mod pinned to a commit of a repository nobody has allowed yet. */
const sphere: MutatorView = {
	name: 'github-dev-sphere-9108a17078f7.sdd',
	title: 'sphere spawner mod v1.0.0',
	description: null,
	here: false,
	check: null,
	source: `github:dev/sphere@${SHA}`,
	date: '2025-10-05T13:32:07Z',
	key: `github:dev/sphere@${SHA}`,
	standing: {
		kind: 'asks',
		why: 'repo',
		text: 'A repository you have not allowed',
	},
}

/** A mod the host loads by name alone. */
const tiny: MutatorView = {
	name: 'tiny maps v1',
	title: 'tiny maps v1',
	description: null,
	here: true,
	check: null,
	source: null,
	date: null,
	key: 'name:tiny maps v1',
	standing: {
		kind: 'asks',
		why: 'unpinned',
		text: 'Loaded by name, from no repository: allowed for one lobby at a time',
	},
}

/** A commit GitHub put off its branch. */
const fork: MutatorView = {
	...sphere,
	name: 'github-dev-tanks-0000000000000.sdd',
	title: 'dev/tanks',
	source: `github:dev/tanks@${'0'.repeat(40)}`,
	key: `github:dev/tanks@${'0'.repeat(40)}`,
	standing: {
		kind: 'asks',
		why: 'branch',
		text: 'not on main of dev/tanks: ahead',
	},
}

const fine: MutatorView = {
	...tiny,
	name: 'fine v1',
	title: 'fine v1',
	key: 'name:fine v1',
	standing: { kind: 'trusted', sandboxed: true },
}

afterEach(() => {
	cleanup()
	setSettingsSignal(null)
	vi.mocked(invoke).mockReset()
})

function card(mutators: MutatorView[]) {
	const { container } = render(() => (
		<ModsConsent mutators={mutators} room={5} />
	))
	const rows = () =>
		[...container.querySelectorAll('.mods-consent .mod-ask')].map((row) => ({
			title: row.querySelector('.mod-name')?.textContent,
			why: row.querySelector('.mod-problem')?.textContent,
		}))
	const button = (text: string) =>
		[...container.querySelectorAll('.mods-consent button')].find((b) =>
			b.textContent?.includes(text),
		) as HTMLButtonElement | undefined
	return { container, rows, button }
}

test('shows one row per mod that asks, with why, and nothing when none does', () => {
	const { container, rows } = card([sphere, tiny, fine])
	expect(container.querySelector('.mods-consent h3')?.textContent).toBe(
		'These mods need your OK',
	)
	expect(container.textContent).toContain(
		"Mods are code that runs on your machine, inside modlobby's sandbox.",
	)
	expect(rows()).toEqual([
		{
			title: 'sphere spawner mod v1.0.0',
			why: 'A repository you have not allowed',
		},
		{
			title: 'tiny maps v1',
			why: 'Loaded by name, from no repository: allowed for one lobby at a time',
		},
	])
	// Where it comes from, to read before allowing it.
	expect(
		container.querySelector('.mods-consent .mod-link')?.textContent,
	).toContain('dev/sphere @ 9108a17')

	const quiet = card([fine])
	expect(quiet.container.querySelector('.mods-consent')).toBeNull()
})

test('Allow sends the repository asks to trustMods and applies the settings it returns', async () => {
	const written = {
		mods: { trusted: [{ repo: 'dev/sphere' }], perCommit: false },
	} as unknown as Settings
	vi.mocked(invoke).mockResolvedValue(written as never)
	const { button } = card([sphere, tiny])
	fireEvent.click(button('Allow')!)
	await waitFor(() =>
		expect(invoke).toHaveBeenCalledWith('trust_mods', {
			asks: [{ repo: 'dev/sphere', commit: SHA }],
		}),
	)
	await waitFor(() => expect(settings()).toEqual(written))
	// The by-name mod is not a repository: it is not what Allow sends.
	expect(invoke).not.toHaveBeenCalledWith('allow_mods_once', expect.anything())
})

test('Allow in this lobby sends the keys of the by-name and off-branch mods to allowModsOnce', async () => {
	vi.mocked(invoke).mockResolvedValue(undefined as never)
	const { button } = card([sphere, tiny, fork])
	fireEvent.click(button('Allow in this lobby')!)
	await waitFor(() =>
		expect(invoke).toHaveBeenCalledWith('allow_mods_once', {
			room: 5,
			keys: [tiny.key, fork.key],
		}),
	)
	expect(invoke).not.toHaveBeenCalledWith('trust_mods', expect.anything())
})

test('the chip says the mods need an OK', () => {
	const asking = render(() => <ModsChip mods={[sphere, fine]} />)
	expect(asking.container.textContent).toContain('needs your OK')
	cleanup()
	const settled = render(() => <ModsChip mods={[fine]} />)
	expect(settled.container.textContent).not.toContain('needs your OK')
})
