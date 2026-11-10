import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { Kind } from '../ipc/bindings/Kind'
import { fixtureOptions } from '../lib/setup.fixture'
import { type DiffSide, SettingsDiff } from './SettingsDiff'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
// Monaco is not what is under test: the diff it would be handed is.
vi.mock('../views/VoteDiff', () => ({
	VoteDiff: (props: { kind: Kind; current: string; proposed: string }) => (
		<div
			data-testid='vote-diff'
			data-kind={props.kind}
			data-current={props.current}
			data-proposed={props.proposed}
		/>
	),
}))

afterEach(cleanup)

/** Lets an awaited answer reach the component and the DOM. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

const side = (
	title: string,
	values: Record<string, string>,
	map = 'Forge',
	blob: DiffSide['blob'] = async () => '',
): DiffSide => ({ title, side: { map, values }, blob })

const BLOBS = {
	aa: { bytes: 4300, name: 'Sphere spawner' },
	bb: { bytes: 1024, name: null },
}

function draw(left: DiffSide, right: DiffSide) {
	return render(() => (
		<SettingsDiff
			catalogue={fixtureOptions()}
			left={left}
			right={right}
			blobs={BLOBS}
		/>
	))
}

const texts = (root: ParentNode, selector: string) =>
	[...root.querySelectorAll(selector)].map((node) => node.textContent?.trim())

describe('two settings side by side', () => {
	test('say so when nothing differs', () => {
		const { container } = draw(
			side('Room', { startmetal: '1000' }),
			side('Then', {}),
		)
		expect(container.textContent).toContain('The same setting')
	})

	test('show what differs in words, under its tab, against the default', () => {
		const { container } = draw(
			side('Room', { startmetal: '2000' }),
			side('Then', {}),
		)
		expect(texts(container, '.settings-diff-section')).toEqual(['Cheats'])
		const row = container.querySelector('.settings-diff-row:not(.head)')
		expect(texts(row as ParentNode, ':scope > span')).toEqual([
			'startmetal',
			'2000',
			'1000',
		])
	})

	test('show the map when it differs', () => {
		const { container } = draw(
			side('Room', {}, 'Forge'),
			side('Then', {}, 'Comet'),
		)
		expect(container.textContent).toContain('Comet')
		expect(container.textContent).not.toContain('The same setting')
	})

	test('name a tweak and its size rather than its blob', () => {
		const { container } = draw(
			side('Room', { tweakdefs: 'aa' }),
			side('Then', { tweakdefs: 'bb' }),
		)
		expect(container.textContent).toContain('Sphere spawner · 4.2 KB')
		expect(container.textContent).toContain('tweak · 1.0 KB')
	})

	test('open a tweak into a Lua diff of both sides', async () => {
		const left = vi.fn(async () => 'LEFT')
		const { container, getByTitle, findByTestId } = draw(
			side('Room', { tweakunits: 'aa' }, 'Forge', left),
			side('Then', {}),
		)
		expect(container.querySelector('[data-testid=vote-diff]')).toBeNull()

		fireEvent.click(getByTitle('Show the Lua that differs'))
		await settle()

		const diff = await findByTestId('vote-diff')
		expect(left).toHaveBeenCalledWith('tweakunits')
		expect(diff.dataset.kind).toBe('units')
		expect(diff.dataset.current).toBe('LEFT')
		expect(diff.dataset.proposed).toBe('')
	})

	test('say so in the Lua pane when a blob cannot be read', async () => {
		const { container, getByTitle } = draw(
			side('Room', { tweakunits: 'aa' }, 'Forge', async () => {
				throw new Error('the blob is gone')
			}),
			side('Then', {}),
		)
		fireEvent.click(getByTitle('Show the Lua that differs'))
		await settle()
		expect(
			container.querySelector('.settings-diff-lua')?.textContent,
		).toContain('the blob is gone')
	})
})
