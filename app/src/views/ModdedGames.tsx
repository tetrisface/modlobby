import { For, Show, createResource, createSignal } from 'solid-js'
import { createStore } from 'solid-js/store'
import { openExternal } from '../components/Linkify'
import { Row } from '../components/SettingRow'
import type { FoldKinds } from '../ipc/bindings/FoldKinds'
import type { KeyChange } from '../ipc/bindings/KeyChange'
import type { ModSetView } from '../ipc/bindings/ModSetView'
import type { TextChange } from '../ipc/bindings/TextChange'
import { api, describeError } from '../ipc/client'
import { age } from '../lib/age'
import { pushNotice } from '../store/chat'

/**
 * What modded games left changed of your settings, kept per set of mods
 * and shown against what each game was given: engine keys, widgets turned
 * on or off and their settings, keybinds, hotkeys and blueprints. Each set
 * folds what you pick into your own settings, or is put back to what its
 * last launch had. Nothing a mod wrote crosses as bytes: every file goes
 * through modlobby's own readers and writers, and a text-valued engine key
 * -- a path, an address -- is shown but never folded.
 */
export function ModdedGamesRows() {
	const [sets, { mutate }] = createResource(async () => {
		try {
			return await api.modSettings()
		} catch (error) {
			pushNotice('warning', describeError(error))
			return []
		}
	})
	return (
		<Show when={!sets.loading}>
			<For
				each={sets() ?? []}
				fallback={
					<Row>
						<p class='muted'>
							No modded game has left settings behind yet. Settings changed
							inside a modded game are kept with that set of mods and listed
							here.
						</p>
					</Row>
				}
			>
				{(set) => <ModSet set={set} onChanged={mutate} />}
			</For>
		</Show>
	)
}

/** Which kinds a set has changes of: the ones worth a checkbox. */
function changed(set: ModSetView): FoldKinds {
	return {
		engine: set.engine.length > 0,
		widgets: set.widgets.length > 0,
		keybinds: set.keybinds !== null,
		hotkeys: set.hotkeys !== null,
		blueprints: set.blueprints !== null,
	}
}

const KINDS: readonly [keyof FoldKinds, string][] = [
	['engine', 'Engine settings'],
	['widgets', 'Widgets'],
	['keybinds', 'Keybinds'],
	['hotkeys', 'Hotkeys'],
	['blueprints', 'Blueprints'],
]

/** One set of mods: its changes since the last fold-in, and the two ways out. */
function ModSet(props: {
	set: ModSetView
	onChanged: (sets: ModSetView[]) => void
}) {
	const has = () => changed(props.set)
	const anything = () => Object.values(has()).some(Boolean)
	const [wanted, setWanted] = createStore<FoldKinds>(changed(props.set))
	const [busy, setBusy] = createSignal(false)
	const asked = (): FoldKinds => ({
		engine: wanted.engine && has().engine,
		widgets: wanted.widgets && has().widgets,
		keybinds: wanted.keybinds && has().keybinds,
		hotkeys: wanted.hotkeys && has().hotkeys,
		blueprints: wanted.blueprints && has().blueprints,
	})

	async function run(ask: () => Promise<ModSetView[]>) {
		setBusy(true)
		try {
			props.onChanged(await ask())
		} catch (error) {
			pushNotice('warning', describeError(error))
		} finally {
			setBusy(false)
		}
	}

	return (
		<Row class='mod-set'>
			<div class='mod-set-head'>
				<For each={props.set.repos}>{(repo) => <Repo repo={repo} />}</For>
				<span class='muted'>loaded as {props.set.order.join(', ')}</span>
				<span class='muted'>played {age(props.set.at)}</span>
			</div>
			<Show
				when={anything()}
				fallback={<p class='muted'>Nothing changed in the last game.</p>}
			>
				<ul class='mod-set-changes'>
					<Show when={has().engine}>
						<li>
							Engine
							<ul>
								<For each={props.set.engine}>
									{(change) => <Change change={change} />}
								</For>
							</ul>
						</li>
					</Show>
					<Show when={has().widgets}>
						<li>
							Widgets
							<ul>
								<For each={props.set.widgets}>
									{(widget) => (
										<li>
											{widget.widget}: {widget.on ? 'off → on' : 'on → off'}
										</li>
									)}
								</For>
							</ul>
						</li>
					</Show>
					<Lines label='Keybinds' change={props.set.keybinds} />
					<Lines label='Hotkeys' change={props.set.hotkeys} />
					<Lines label='Blueprints' change={props.set.blueprints} />
				</ul>
				<Show when={has().widgets}>
					<p class='muted'>
						Widget settings changed in these games stay with this set of mods: a
						modded game starts every widget at its defaults, so what it left
						cannot tell your change from a default.
					</p>
				</Show>
				<div class='mod-set-fold'>
					<For each={KINDS.filter(([kind]) => has()[kind])}>
						{([kind, label]) => (
							<label class='row'>
								<input
									type='checkbox'
									checked={wanted[kind]}
									onChange={(e) => setWanted(kind, e.currentTarget.checked)}
								/>
								{label}
							</label>
						)}
					</For>
					<button
						type='button'
						class='primary'
						disabled={busy() || !Object.values(asked()).some(Boolean)}
						title='Write the chosen kinds into your own settings'
						onClick={() =>
							void run(() => api.foldInModSettings(props.set.id, asked()))
						}
					>
						Fold in
					</button>
					<button
						type='button'
						disabled={busy()}
						title="Forget what this set's games changed; its next game starts from your own settings"
						onClick={() => void run(() => api.discardModSettings(props.set.id))}
					>
						Discard
					</button>
				</div>
			</Show>
		</Row>
	)
}

/**
 * A file's lines the games added and removed, as they would land in yours:
 * a binding is an action at a key press, so each is shown before it folds.
 */
function Lines(props: { label: string; change: TextChange | null }) {
	return (
		<Show when={props.change}>
			{(change) => (
				<li>
					{props.label}
					<ul>
						<For each={change().added}>{(line) => <li>+ {line}</li>}</For>
						<For each={change().removed}>{(line) => <li>− {line}</li>}</For>
						<Show when={change().more > 0}>
							<li class='muted'>and {change().more} more</li>
						</Show>
					</ul>
				</li>
			)}
		</Show>
	)
}

/** A repository on GitHub, or a mod loaded by name alone. */
function Repo(props: { repo: string }) {
	const named = () => props.repo.startsWith('name:')
	return (
		<Show
			when={!named()}
			fallback={<span class='mod-set-repo'>{props.repo.slice(5)}</span>}
		>
			<button
				type='button'
				class='mod-link mod-set-repo'
				title='On GitHub'
				onClick={() => void openExternal(`https://github.com/${props.repo}`)}
			>
				{props.repo}
			</button>
		</Show>
	)
}

/**
 * One key's change. A text value is shown and never folded: it could be
 * a path or an address, so it is yours to set by hand if you want it.
 */
function Change(props: { change: KeyChange }) {
	return (
		<li
			class={props.change.plain ? undefined : 'by-hand'}
			title={
				props.change.plain
					? undefined
					: 'A text value could be a path or an address: not folded in'
			}
		>
			{props.change.key}: {props.change.before ?? '—'} → {props.change.after}
			<Show when={!props.change.plain}>
				<span class='muted'> set by hand</span>
			</Show>
		</li>
	)
}
