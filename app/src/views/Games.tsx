import { For, Show } from 'solid-js'
import { reconcile, type SetStoreFunction } from 'solid-js/store'
import { Select } from '../components/Select'
import { Row } from '../components/SettingRow'
import type { GameOverride } from '../ipc/bindings/GameOverride'
import type { GameSource } from '../ipc/bindings/GameSource'
import type { Settings } from '../ipc/bindings/Settings'

/** A source of `kind`, nothing filled in yet. */
function fresh(kind: string): GameSource {
	switch (kind) {
		case 'url':
			return { kind: 'url', value: '' }
		case 'git':
			return { kind: 'git', value: '' }
		default:
			return { kind: 'github', value: '' }
	}
}

/** An override just added, to be filled in. */
const blank = (): GameOverride => ({
	name: '',
	source: { kind: 'github', value: '' },
})

/**
 * Where games come from when a room's server does not have them, and the
 * player's own word on single games.
 *
 * An override is strict on purpose: exactly one name, and nothing else is
 * tried for it, not even the server's rapid. That is what makes it safe to
 * leave in place for good -- it can never catch a version it was not
 * written for.
 */
export function GameOverrideRows(props: {
	draft: Settings
	setDraft: SetStoreFunction<Settings>
}) {
	// Replaced whole: a store merges an object into the one it holds, and a
	// source of one kind carries fields another kind has no use for.
	const change = (index: number, source: GameSource) =>
		props.setDraft('games', 'overrides', index, 'source', reconcile(source))

	return (
		<>
			<Row>
				<p class='muted'>
					A game the room's server does not have is looked for in modlobby's
					list, then in coilbox's hub, and fetched from where they point: a
					GitHub release, an address, or a commit built here -- which is kept
					only once it is exactly the room's copy. An override below goes first,
					for exactly the game it names and no other version, ahead of the
					server's own rapid.
				</p>
			</Row>
			<For each={props.draft.games.overrides}>
				{(kept, index) => (
					<Row>
						<div class='game-override'>
							<label>
								Game, exactly as rooms name it
								<input
									value={kept.name}
									placeholder='SplinterFaction 0.1.86'
									onInput={(e) =>
										props.setDraft(
											'games',
											'overrides',
											index(),
											'name',
											e.currentTarget.value,
										)
									}
								/>
							</label>
							<label>
								From
								<Select
									value={kept.source.kind}
									onChange={(e) =>
										change(index(), fresh(e.currentTarget.value))
									}
								>
									<option value='github'>GitHub releases</option>
									<option value='git'>A git commit, built</option>
									<option value='url'>An address</option>
								</Select>
							</label>
							<label>
								{kept.source.kind === 'url' ? 'Address' : 'Repository'}
								<input
									value={kept.source.value}
									placeholder={
										kept.source.kind === 'url'
											? 'https://…/game.sdz'
											: 'owner/repo'
									}
									onInput={(e) =>
										change(index(), {
											...kept.source,
											value: e.currentTarget.value,
										})
									}
								/>
							</label>
							<Show when={kept.source.kind === 'github'}>
								<label>
									File name has (optional)
									<input
										value={
											kept.source.kind === 'github'
												? (kept.source.asset ?? '')
												: ''
										}
										onInput={(e) =>
											change(index(), {
												kind: 'github',
												value: kept.source.value,
												asset: e.currentTarget.value || null,
											})
										}
									/>
								</label>
							</Show>
							<Show when={kept.source.kind === 'git'}>
								<label>
									Version placeholder (optional)
									<input
										value={
											kept.source.kind === 'git'
												? (kept.source.placeholder ?? '')
												: ''
										}
										placeholder='$VERSION'
										onInput={(e) =>
											change(index(), {
												kind: 'git',
												value: kept.source.value,
												placeholder: e.currentTarget.value || null,
											})
										}
									/>
								</label>
							</Show>
							<button
								type='button'
								class='link'
								onClick={() =>
									props.setDraft('games', 'overrides', (all) =>
										all.filter((_, at) => at !== index()),
									)
								}
							>
								Remove
							</button>
						</div>
					</Row>
				)}
			</For>
			<Row>
				<button
					type='button'
					onClick={() =>
						props.setDraft('games', 'overrides', (all) => [...all, blank()])
					}
				>
					Add an override
				</button>
			</Row>
		</>
	)
}
