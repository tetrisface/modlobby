import { For, Show, createMemo, createSignal } from 'solid-js'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import type { TrustAsk } from '../ipc/bindings/TrustAsk'
import { api, describeError } from '../ipc/client'
import { pinOf, sourceWords } from '../lib/mutators'
import { pushNotice } from '../store/chat'
import { applySettings } from '../store/settings'
import { openExternal } from './Linkify'

/**
 * The lobby's mods that have not been allowed yet, each with why it asks,
 * and the two ways to allow them: remembered for the repository, sandboxed,
 * through Settings; or for this lobby alone, for a mod with nothing to
 * remember it by. Nothing while every mod runs.
 */
export function ModsConsent(props: {
	mutators: readonly MutatorView[]
	/** The lobby the card is shown in: an allowance is given for it alone. */
	room: number
}) {
	const asking = createMemo(() =>
		props.mutators.filter((mod) => mod.standing.kind === 'asks'),
	)
	/** A new repository, or a new commit of one: remembered once allowed. */
	const remembered = () =>
		asking().filter(
			(mod) =>
				mod.standing.kind === 'asks' &&
				(mod.standing.why === 'repo' || mod.standing.why === 'commit'),
		)
	/** By name, or a commit off its branch: allowed for this lobby only. */
	const once = () =>
		asking().filter(
			(mod) =>
				mod.standing.kind === 'asks' &&
				(mod.standing.why === 'unpinned' || mod.standing.why === 'branch'),
		)
	const [busy, setBusy] = createSignal(false)

	async function allow() {
		const asks: TrustAsk[] = remembered().flatMap((mod) => {
			const pin = pinOf(mod.source)
			return pin ? [{ repo: pin.repo, commit: pin.commit }] : []
		})
		if (asks.length === 0) return
		setBusy(true)
		try {
			applySettings(await api.trustMods(asks))
		} catch (error) {
			pushNotice('warning', describeError(error))
		} finally {
			setBusy(false)
		}
	}

	async function allowHere() {
		const keys = once().map((mod) => mod.key)
		if (keys.length === 0) return
		setBusy(true)
		try {
			await api.allowModsOnce(props.room, keys)
		} catch (error) {
			pushNotice('warning', describeError(error))
		} finally {
			setBusy(false)
		}
	}

	return (
		<Show when={asking().length > 0}>
			<section class='mods-consent'>
				<h3>These mods need your OK</h3>
				<p class='muted'>
					Mods are code that runs on your machine, inside modlobby's sandbox.
				</p>
				<For each={asking()}>{(mod) => <Ask mod={mod} />}</For>
				<div class='mods-ways'>
					<Show when={remembered().length > 0}>
						<button
							class='primary'
							disabled={busy()}
							title='Allow these repositories, sandboxed; kept in Settings › Mods'
							onClick={() => void allow()}
						>
							Allow
						</button>
					</Show>
					<Show when={once().length > 0}>
						<button
							disabled={busy()}
							title='Allow these in this lobby only, sandboxed'
							onClick={() => void allowHere()}
						>
							Allow in this lobby
						</button>
					</Show>
				</div>
			</section>
		</Show>
	)
}

/** One mod that asks: what it is, where it comes from, and why it asks. */
function Ask(props: { mod: MutatorView }) {
	const pin = () => pinOf(props.mod.source)
	return (
		<div class='mod-ask'>
			<span class='mod-name'>{props.mod.title}</span>
			<Show
				when={pin()}
				fallback={<span class='mod-source muted'>{props.mod.name}</span>}
			>
				{(held) => (
					<button
						type='button'
						class='mod-link'
						title='On GitHub, to read before allowing it'
						onClick={() =>
							void openExternal(
								`https://github.com/${held().repo}/commit/${held().commit}`,
							)
						}
					>
						{sourceWords(props.mod.source ?? '')}
					</button>
				)}
			</Show>
			<span class='mod-problem'>
				{props.mod.standing.kind === 'asks' ? props.mod.standing.text : ''}
			</span>
		</div>
	)
}
