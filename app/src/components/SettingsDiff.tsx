import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import type { BlobInfo } from '../ipc/bindings/BlobInfo'
import type { ModOption } from '../ipc/bindings/ModOption'
import { describeError } from '../ipc/client'
import { type DiffRow, type Side, diffSections } from '../lib/history'
import { defaultText, isCleared, tweakKey, wordsFor } from '../lib/setup'
import { VoteDiff } from '../views/VoteDiff'
import { CellButton } from './ActionCell'
import { OptionName } from './OptionName'

/** One side of a comparison, and where its tweaks' Lua comes from. */
export type DiffSide = {
	title: string
	side: Side
	/** The blob in a tweak slot on this side. */
	blob: (key: string) => Promise<string>
}

/** A slot value in words: the tweak's name and size, never its blob. */
function tweakWords(
	value: string | null,
	blobs: Record<string, BlobInfo>,
): string {
	if (value === null || isCleared(value)) return 'empty'
	const info = blobs[value]
	if (!info) return 'tweak'
	return `${info.name ?? 'tweak'} · ${(info.bytes / 1024).toFixed(1)} KB`
}

function words(
	row: DiffRow,
	value: string | null,
	blobs: Record<string, BlobInfo>,
): string {
	if (tweakKey(row.key) !== null) return tweakWords(value, blobs)
	if (!row.option) return value ?? 'not set'
	return wordsFor(row.option, value ?? defaultText(row.option))
}

/**
 * What differs between two settings, under the tabs a room shows: a row per
 * option, its value on each side in words. A tweak slot opens into a diff of
 * its Lua, fetched only then.
 */
export function SettingsDiff(props: {
	catalogue: readonly ModOption[]
	left: DiffSide
	right: DiffSide
	blobs: Record<string, BlobInfo>
}) {
	const sections = createMemo(() =>
		diffSections(props.catalogue, props.left.side, props.right.side),
	)
	const mapDiffers = () => props.left.side.map !== props.right.side.map
	const [open, setOpen] = createSignal<string | null>(null)

	return (
		<div class='settings-diff'>
			<div class='settings-diff-row head'>
				<span>Setting</span>
				<span title={props.left.title}>{props.left.title}</span>
				<span title={props.right.title}>{props.right.title}</span>
			</div>
			<Show when={mapDiffers()}>
				<div class='settings-diff-row'>
					<span>Map</span>
					<span>{props.left.side.map ?? 'none'}</span>
					<span>{props.right.side.map ?? 'none'}</span>
				</div>
			</Show>
			<For each={sections()}>
				{(section) => (
					<>
						<Show when={section.name}>
							<div class='settings-diff-section'>{section.name}</div>
						</Show>
						<For each={section.rows}>
							{(row) => {
								const kind = tweakKey(row.key)?.kind
								return (
									<>
										<div
											class='settings-diff-row'
											classList={{ tweak: !!kind }}
										>
											<span class='settings-diff-name'>
												{row.option ? (
													<OptionName option={row.option} />
												) : (
													row.key
												)}
												<Show when={kind}>
													<CellButton
														icon='act-expand'
														title='Show the Lua that differs'
														onClick={() =>
															setOpen(open() === row.key ? null : row.key)
														}
													/>
												</Show>
											</span>
											<span>{words(row, row.left, props.blobs)}</span>
											<span>{words(row, row.right, props.blobs)}</span>
										</div>
										<Show when={kind && open() === row.key}>
											<TweakDiff
												row={row}
												kind={kind ?? 'defs'}
												left={props.left}
												right={props.right}
											/>
										</Show>
									</>
								)
							}}
						</For>
					</>
				)}
			</For>
			<Show when={!mapDiffers() && sections().length === 0}>
				<p class='muted setup-empty'>The same setting.</p>
			</Show>
		</div>
	)
}

/** One slot's Lua on both sides, an empty slot being no Lua at all. */
function TweakDiff(props: {
	row: DiffRow
	kind: 'defs' | 'units'
	left: DiffSide
	right: DiffSide
}) {
	const text = (side: DiffSide, value: string | null) =>
		value === null || isCleared(value)
			? Promise.resolve('')
			: side.blob(props.row.key)
	// Caught here rather than read as an errored resource, which would throw
	// into the pane: a blob gone from the disk is something to say, not a crash.
	const [pair] = createResource(async () => {
		try {
			const [current, proposed] = await Promise.all([
				text(props.left, props.row.left),
				text(props.right, props.row.right),
			])
			return { current, proposed, error: null }
		} catch (error) {
			return { current: '', proposed: '', error: describeError(error) }
		}
	})
	return (
		<div class='settings-diff-lua'>
			<Show when={pair()} fallback={<p class='muted'>Reading the Lua…</p>}>
				{(both) => (
					<Show
						when={both().error === null}
						fallback={
							<p class='error'>The Lua could not be read: {both().error}</p>
						}
					>
						<VoteDiff
							kind={props.kind}
							current={both().current}
							proposed={both().proposed}
							title={props.row.key}
						/>
					</Show>
				)}
			</Show>
		</div>
	)
}
