import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import type { Prepared } from '../../ipc/bindings/Prepared'
import { BOX_OVERRIDE } from '../../lib/boxes'
import {
	SCRATCH,
	draftDoc,
	edit,
	emptyWorkspace,
	slotId,
	type Doc,
} from '../../lib/tweakspace'
import { SendBar, Toolbar } from './Toolbar'

const clean = emptyWorkspace().docs[slotId('tweakdefs1')]!
const typed = edit(clean, 'local a = 1')

const prepared = (fits: boolean): Prepared => ({
	minified: 'local a=1',
	blob: 'bG9jYWwgYT0x',
	command: '!bSet tweakdefs1 bG9jYWwgYT0x',
	gauge: {
		raw: 11,
		minified: 9,
		// The command is the blob and the 18 characters of `!bSet tweakdefs1 `.
		blob: fits ? 12 : 19982,
		command: fits ? 30 : 20000,
		cap: 16385,
		fits,
	},
})

function toolbar(doc: Doc, over: Record<string, unknown> = {}) {
	const on = {
		onFormat: vi.fn(),
		onReset: vi.fn(),
		onSave: vi.fn(),
		onLoad: vi.fn(),
		onDelete: vi.fn(),
		onFullscreen: vi.fn(),
		onCompare: vi.fn(),
		onCopy: vi.fn(),
		onSearch: vi.fn(),
		onPalette: vi.fn(),
		onDone: vi.fn(),
	}
	const view = render(() => (
		<Toolbar
			doc={doc}
			prepared={null}
			busy={false}
			fullscreen={false}
			comparing={false}
			searching={false}
			closeTitle='Fold the editor away'
			heading={false}
			drafts={[]}
			{...on}
			{...over}
		/>
	))
	return { on, ...view }
}

function sendBar(doc: Doc, over: Record<string, unknown> = {}) {
	const on = {
		onMinify: vi.fn(),
		onTarget: vi.fn(),
		onSend: vi.fn(),
		onClear: vi.fn(),
		onCopy: vi.fn(),
	}
	const view = render(() => (
		<SendBar
			doc={doc}
			prepared={prepared(true)}
			problem={null}
			busy={false}
			refusal={null}
			spads={true}
			target='tweakdefs1'
			minify={false}
			{...on}
			{...over}
		/>
	))
	return { on, ...view }
}

const button = (element: HTMLElement) => element as HTMLButtonElement

describe('Toolbar', () => {
	test('a clean slot cannot be reset; an edited one can', () => {
		const first = toolbar(clean)
		expect(button(first.getByText('Reset')).disabled).toBe(true)
		first.unmount()

		const second = toolbar(typed)
		fireEvent.click(second.getByText('Reset'))
		expect(second.on.onReset).toHaveBeenCalled()
	})

	test('under a row it names nothing; the drafts editor names the document and goes back', () => {
		const row = toolbar(typed)
		expect(row.queryByText('tweakdefs1')).toBeNull()
		expect(row.queryByText('‹ Settings')).toBeNull()
		row.unmount()

		const onClose = vi.fn()
		const desk = toolbar(typed, { heading: true, onClose })
		expect(desk.getByText('tweakdefs1')).toBeTruthy()
		expect(desk.getByText('edited')).toBeTruthy()
		fireEvent.click(desk.getByText('‹ Settings'))
		expect(onClose).toHaveBeenCalled()
	})

	test('the copy menu hands out each form, the prepared ones once there are some', () => {
		const idle = toolbar(typed)
		fireEvent.click(idle.getByText('Copy'))
		expect(button(idle.getByText('minified')).disabled).toBe(true)
		fireEvent.click(idle.getByText('Lua'))
		expect(idle.on.onCopy).toHaveBeenCalledWith('lua')
		// Picking one closes the menu.
		expect(idle.queryByText('minified')).toBeNull()
		idle.unmount()

		const ready = toolbar(typed, { prepared: prepared(true) })
		fireEvent.click(ready.getByText('Copy'))
		fireEvent.click(ready.getByText('!bSet command'))
		expect(ready.on.onCopy).toHaveBeenLastCalledWith('command')
		fireEvent.click(ready.getByText('Copy'))
		fireEvent.click(ready.getByText('base64url'))
		expect(ready.on.onCopy).toHaveBeenLastCalledWith('blob')
	})

	test('the drafts menu saves under the typed name or the header, loads, and deletes', () => {
		const drafts = [
			{ title: 'walls', name: 'T3 walls' },
			{ title: 'nukes', name: null },
		]
		const { on, getByText, getByLabelText } = toolbar(
			{ ...typed, name: 'Nutty B' },
			{ drafts },
		)
		fireEvent.click(getByText('Drafts'))
		fireEvent.click(getByText('Save draft'))
		expect(on.onSave).toHaveBeenLastCalledWith('Nutty B')

		fireEvent.click(getByText('Drafts'))
		fireEvent.input(getByLabelText('Draft name'), {
			target: { value: 'walls-2' },
		})
		fireEvent.click(getByText('Save draft'))
		expect(on.onSave).toHaveBeenLastCalledWith('walls-2')

		fireEvent.click(getByText('Drafts'))
		expect(getByText('Load into tweakdefs1')).toBeTruthy()
		fireEvent.click(getByText('walls'))
		expect(on.onLoad).toHaveBeenCalledWith('walls')

		fireEvent.click(getByText('Drafts'))
		fireEvent.click(getByLabelText('Delete the draft nukes'))
		expect(on.onDelete).toHaveBeenCalledWith('nukes')
	})

	test('Save keeps it as a draft under its name, as Ctrl+S does', () => {
		const { on, getByText } = toolbar({ ...typed, name: 'Nutty B' })
		fireEvent.click(getByText('Save'))
		expect(on.onSave).toHaveBeenCalledWith('Nutty B')
	})

	test('a draft is saved under its own name', () => {
		const { on, getByText } = toolbar(draftDoc('walls', '-- T3 walls\n{}'))
		fireEvent.click(getByText('Drafts'))
		fireEvent.click(getByText('Save draft'))
		expect(on.onSave).toHaveBeenLastCalledWith('walls')
	})

	test('compare, search, the palette and the corner buttons go to the caller', () => {
		const { on, getByText, getByLabelText } = toolbar(typed)
		fireEvent.click(getByLabelText('Fill the window'))
		expect(on.onFullscreen).toHaveBeenCalledWith(true)
		fireEvent.click(getByText('Compare'))
		expect(on.onCompare).toHaveBeenCalled()
		fireEvent.click(getByText('Search all'))
		expect(on.onSearch).toHaveBeenCalled()
		fireEvent.click(getByText('Command palette'))
		expect(on.onPalette).toHaveBeenCalled()
		fireEvent.click(getByLabelText('Fold the editor away'))
		expect(on.onDone).toHaveBeenCalled()
	})

	test('filling the window turns the corner into the way back', () => {
		const { on, getByLabelText } = toolbar(typed, { fullscreen: true })
		fireEvent.click(getByLabelText('Back into the pane'))
		expect(on.onFullscreen).toHaveBeenCalledWith(false)
	})

	test('the start-box override copies as JSON and offers no draft', () => {
		const boxes = emptyWorkspace().docs[slotId(BOX_OVERRIDE)]!
		const { on, getByText, queryByText } = toolbar(boxes)
		expect(queryByText('Drafts')).toBeNull()
		expect(queryByText('Save'), 'no drafts, so nothing to save to').toBeNull()
		fireEvent.click(getByText('Copy'))
		expect(queryByText('Lua')).toBeNull()
		// The wire form is zlib inside the base64url, and the button says so.
		expect(queryByText('base64url')).toBeNull()
		expect(
			button(getByText('base64url+zlib')).disabled,
			'nothing prepared yet',
		).toBe(true)
		fireEvent.click(getByText('JSON'))
		expect(on.onCopy).toHaveBeenCalledWith('lua')
	})
})

describe('SendBar', () => {
	test('sends, votes and clears for someone with a seat', () => {
		const { on, getByText } = sendBar(typed)
		fireEvent.click(getByText('Send !bSet'))
		expect(on.onSend).toHaveBeenCalledWith(true)
		fireEvent.click(getByText('Call a vote'))
		expect(on.onSend).toHaveBeenCalledWith(false)
		fireEvent.click(getByText('Clear slot'))
		expect(on.onClear).toHaveBeenCalled()
	})

	test('a slot as the room holds it has nothing to send; a draft does', () => {
		const slot = sendBar(clean)
		expect(button(slot.getByText('Send !bSet')).disabled).toBe(true)
		expect(button(slot.getByText('Call a vote')).disabled).toBe(true)
		slot.unmount()

		const draft = sendBar(draftDoc('walls', '{}'), { target: 'tweakunits1' })
		expect(button(draft.getByText('Send !bSet')).disabled).toBe(false)
	})

	test('says how long each form is, and how long the base64url may be', () => {
		const { container, getByText } = sendBar(typed)
		expect(getByText('lua 11')).toBeTruthy()
		expect(getByText('minified 9')).toBeTruthy()
		expect(getByText('base64url 12')).toBeTruthy()
		// The server's 16385, less the `!bSet tweakdefs1 ` in front.
		expect(getByText('max 16367')).toBeTruthy()
		expect(container.querySelector('.gauge .over')).toBeNull()
	})

	test('each number over the max goes red on its own, and nothing is sent', () => {
		const long = prepared(false)
		long.gauge = { ...long.gauge, raw: 20000, minified: 15000 }
		const { getByText } = sendBar(typed, { prepared: long })
		const red = (text: string) => getByText(text).closest('.over') !== null
		expect(red('lua 20000')).toBe(true)
		expect(red('minified 15000'), 'minifying would fit').toBe(false)
		expect(red('base64url 19982')).toBe(true)
		expect(button(getByText('Send !bSet')).disabled).toBe(true)
		expect(button(getByText('Call a vote')).disabled).toBe(true)
	})

	test('the override is compressed, so only its blob can go red', () => {
		const override = emptyWorkspace().docs[slotId(BOX_OVERRIDE)]!
		const long = prepared(true)
		long.gauge = { ...long.gauge, raw: 20000, minified: 17000 }
		const fits = sendBar(override, { target: BOX_OVERRIDE, prepared: long })
		expect(fits.container.querySelector('.gauge .over')).toBeNull()
		fits.unmount()

		const tooLong = prepared(false)
		tooLong.gauge = { ...tooLong.gauge, raw: 20000, minified: 17000 }
		const { getByText } = sendBar(override, {
			target: BOX_OVERRIDE,
			prepared: tooLong,
		})
		const red = (text: string) => getByText(text).closest('.over') !== null
		expect(red('json 20000')).toBe(false)
		expect(red('minified 17000')).toBe(false)
		expect(red('base64url+zlib 19982')).toBe(true)
	})

	test('minifying is asked for, never assumed; the override has no say', () => {
		const lua = sendBar(typed)
		const box = lua.getByRole('checkbox') as HTMLInputElement
		expect(box.checked, 'sent as written by default').toBe(false)
		fireEvent.click(box)
		expect(lua.on.onMinify).toHaveBeenCalledWith(true)
		lua.unmount()

		const boxes = sendBar(emptyWorkspace().docs[slotId(BOX_OVERRIDE)]!, {
			target: BOX_OVERRIDE,
		})
		expect(boxes.queryByRole('checkbox')).toBeNull()
		expect(boxes.getByText('minified 9')).toBeTruthy()
	})

	test('where the room would refuse it, the buttons stay greyed and say why', () => {
		const why = 'Join as a player to change settings'
		const { on, getByText } = sendBar(typed, { refusal: why })
		expect(getByText(why)).toBeTruthy()
		for (const label of ['Send !bSet', 'Call a vote', 'Clear slot']) {
			expect(button(getByText(label)).disabled, label).toBe(true)
			expect(getByText(label).title, label).toBe(why)
		}
		// The command can still go to somebody who can set it.
		fireEvent.click(getByText('Copy !bSet'))
		expect(on.onCopy).toHaveBeenCalledWith('command')
	})

	test('a room of your own sets the slot, with no vote and no cap to fit', () => {
		const { on, getByText, queryByText } = sendBar(typed, {
			spads: false,
			prepared: prepared(false),
		})
		expect(queryByText('Call a vote')).toBeNull()
		expect(queryByText(/max/)).toBeNull()
		fireEvent.click(getByText('Set slot'))
		expect(on.onSend).toHaveBeenCalledWith(true)
	})

	test('a syntax error is said, and nothing is sent', () => {
		const { getByTitle, getByText } = sendBar(typed, {
			prepared: null,
			problem: 'Lua: unexpected token',
		})
		expect(getByTitle('Lua: unexpected token').textContent).toBe(
			'will not load',
		)
		expect(button(getByText('Send !bSet')).disabled).toBe(true)
	})

	test('a draft picks a slot of its kind; the scratch any tweak slot', () => {
		const draft = sendBar(draftDoc('walls', '{}'), { target: 'tweakunits1' })
		const options = (view: typeof draft) =>
			[...view.getByRole('combobox').querySelectorAll('option')].map(
				(option) => option.value,
			)
		expect(options(draft)).toHaveLength(10)
		expect(options(draft).every((key) => key.startsWith('tweakunits'))).toBe(
			true,
		)
		fireEvent.change(draft.getByRole('combobox'), {
			target: { value: 'tweakunits4' },
		})
		expect(draft.on.onTarget).toHaveBeenCalledWith('tweakunits4')
		draft.unmount()

		const scratch = sendBar(emptyWorkspace().docs[SCRATCH]!)
		expect(options(scratch)).toHaveLength(20)
		scratch.unmount()

		const slot = sendBar(typed)
		expect(slot.queryByRole('combobox')).toBeNull()
	})
})
