import { For, Show } from 'solid-js'
import { reconcile, type SetStoreFunction } from 'solid-js/store'
import { Select } from '../components/Select'
import { Row } from '../components/SettingRow'
import type { GameOverride } from '../ipc/bindings/GameOverride'
import type { GameSource } from '../ipc/bindings/GameSource'
import type { Settings } from '../ipc/bindings/Settings'

/** Each kind of source: what it is called, and what its one field holds. */
const KINDS: Record<
	GameSource['kind'],
	{ label: string; field: string; hint: string }
> = {
	github: { label: 'GitHub releases', field: 'Repository', hint: 'owner/repo' },
	gitlab: {
		label: 'GitLab releases',
		field: 'Project',
		hint: 'group/project, or https://host/group/project',
	},
	forgejo: {
		label: 'Forgejo or Codeberg releases',
		field: 'Repository',
		hint: 'owner/repo, or https://host/owner/repo',
	},
	git: {
		label: 'A GitHub commit, built',
		field: 'Repository',
		hint: 'owner/repo',
	},
	url: { label: 'An address', field: 'Address', hint: 'https://…/game.sdz' },
	rapid: {
		label: "Rapid, from the community's server",
		field: 'Rapid tag',
		hint: 'evo:stable',
	},
}

/** A source of `kind`, nothing filled in yet. */
const fresh = (kind: string) => ({ kind, value: '' }) as GameSource

/** A source that picks among a release's files, when this is one. */
const released = (source: GameSource) =>
	source.kind === 'github' ||
	source.kind === 'gitlab' ||
	source.kind === 'forgejo'
		? source
		: null

/** A commit to build, when this is one. */
const built = (source: GameSource) => (source.kind === 'git' ? source : null)

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
					release on GitHub, GitLab or a Forgejo such as Codeberg, an address,
					the community's rapid server, or a GitHub commit built here -- which
					is kept only once its checksum is the room's. An override below goes
					first, for exactly the game it names and no other version, ahead of
					the server's own rapid.
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
									<For each={Object.entries(KINDS)}>
										{([kind, about]) => (
											<option value={kind}>{about.label}</option>
										)}
									</For>
								</Select>
							</label>
							<label>
								{KINDS[kept.source.kind].field}
								<input
									value={kept.source.value}
									placeholder={KINDS[kept.source.kind].hint}
									onInput={(e) =>
										change(index(), {
											...kept.source,
											value: e.currentTarget.value,
										})
									}
								/>
							</label>
							<Show when={released(kept.source)}>
								{(source) => (
									<label>
										File name has (optional)
										<input
											value={source().asset ?? ''}
											onInput={(e) =>
												change(index(), {
													...source(),
													asset: e.currentTarget.value || null,
												})
											}
										/>
									</label>
								)}
							</Show>
							<Show when={built(kept.source)}>
								{(source) => (
									<label>
										Version placeholder (optional)
										<input
											value={source().placeholder ?? ''}
											placeholder='$VERSION'
											onInput={(e) =>
												change(index(), {
													...source(),
													placeholder: e.currentTarget.value || null,
												})
											}
										/>
									</label>
								)}
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
