import { For, Show, createSignal } from 'solid-js'
import type { SetStoreFunction } from 'solid-js/store'
import { Select } from '../components/Select'
import { Row } from '../components/SettingRow'
import type { Settings } from '../ipc/bindings/Settings'
import type { Trusted } from '../ipc/bindings/Trusted'

/** How an entry's reach reads: the commit it is for, or the branch it follows. */
export const reachWords = (entry: Trusted): string =>
	entry.commit ? entry.commit.slice(0, 7) : 'any commit on its default branch'

const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/

/**
 * Whose mods may run: the repositories allowed from the Mods tab's card or
 * here by hand, each sandboxed unless switched off, and whether trust is
 * given a commit at a time. Revoking one makes its lobbies ask again.
 */
export function ModsTrustRows(props: {
	draft: Settings
	setDraft: SetStoreFunction<Settings>
}) {
	const [repo, setRepo] = createSignal('')
	const valid = () => REPO.test(repo().trim())

	function add() {
		if (!valid()) return
		props.setDraft('mods', 'trusted', (all) => [
			...all,
			{
				repo: repo().trim(),
				commit: null,
				sandboxed: true,
				since: new Date().toISOString(),
			},
		])
		setRepo('')
	}

	return (
		<>
			<Row>
				<label class='row'>
					<input
						type='checkbox'
						checked={props.draft.mods.perCommit}
						onChange={(e) =>
							props.setDraft('mods', 'perCommit', e.currentTarget.checked)
						}
					/>
					Trust per commit
				</label>
				<p class='muted'>
					A repository you allowed asks again at each new commit.
				</p>
			</Row>
			<For
				each={props.draft.mods.trusted}
				fallback={
					<Row>
						<p class='muted'>
							No repositories allowed yet. A lobby's mods ask in its Mods tab,
							and what you allow there is kept here.
						</p>
					</Row>
				}
			>
				{(entry, index) => (
					<Row>
						<div class='mod-trust'>
							<span class='mod-trust-repo'>{entry.repo}</span>
							<span class='muted'>{reachWords(entry)}</span>
							<label>
								Sandbox
								<Select
									value={entry.sandboxed ? 'on' : 'off'}
									onChange={(e) =>
										props.setDraft(
											'mods',
											'trusted',
											index(),
											'sandboxed',
											e.currentTarget.value === 'on',
										)
									}
								>
									<option value='on'>On</option>
									<option value='off'>Off — runs with your rights</option>
								</Select>
							</label>
							<button
								type='button'
								class='link'
								onClick={() =>
									props.setDraft('mods', 'trusted', (all) =>
										all.filter((_, at) => at !== index()),
									)
								}
							>
								Revoke
							</button>
							<Show when={!entry.sandboxed}>
								<p class='mod-trust-warning'>
									Mod code from this repository runs outside the sandbox, as you
									do. Only for mods you have read.
								</p>
							</Show>
						</div>
					</Row>
				)}
			</For>
			<Row>
				<div class='mods-trust-add'>
					<input
						value={repo()}
						placeholder='owner/repo'
						aria-label='Allow a repository'
						onInput={(e) => setRepo(e.currentTarget.value)}
						onKeyDown={(e) => {
							if (e.key === 'Enter') add()
						}}
					/>
					<button type='button' disabled={!valid()} onClick={add}>
						Allow a repository
					</button>
				</div>
				<p class='muted'>
					Allowed by hand, sandboxed, at any commit on its default branch.
				</p>
			</Row>
		</>
	)
}
