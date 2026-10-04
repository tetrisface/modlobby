import { createSignal, type Accessor } from 'solid-js'

/**
 * The mark of an overview beside a long scrolling page: which of the page's
 * sections is being read, and a way to go to one. Settings keeps one in its
 * margin, and the room's options pane one beside its list.
 *
 * The section being read is the last one whose start has scrolled past the
 * read line, or the last of all once the page will not scroll further -- a
 * short final section never reaches the line. While the pointer rests on a
 * section, that one: what a hand rests on is what is being read, and it says
 * so before any scroll does.
 */
export function createOverview<Id extends string>(options: {
	/** The scrolling page. */
	page: () => HTMLElement | undefined
	/** The sections, in page order. */
	ids: Accessor<readonly Id[]>
	/** Where a section starts on the page. */
	start: (id: Id) => HTMLElement | null | undefined
	/** The section an element belongs to, if it belongs to one. */
	sectionOf: (target: Element) => Id | null
	/** How far below the page's top edge the read line runs, in pixels. */
	line: number
}) {
	const [reading, setReading] = createSignal<Id | null>(null)
	const [pointed, setPointed] = createSignal<Id | null>(null)

	/**
	 * Set while the page scrolls to where the overview sent it. That scroll is
	 * not the reader's, and near the foot of the page it stops short of putting
	 * the section at the top, so it would otherwise mark a neighbour instead of
	 * the entry that was pressed. The reader's own wheel takes over at once.
	 */
	let jumping = false

	function spy() {
		const page = options.page()
		if (!page || jumping) return
		const atEnd = page.scrollTop + page.clientHeight >= page.scrollHeight - 1
		const line = page.getBoundingClientRect().top + options.line
		const passed = options.ids().filter((id) => {
			const top = options.start(id)?.getBoundingClientRect().top
			return top !== undefined && (atEnd || top <= line)
		})
		setReading(() => passed.at(-1) ?? null)
	}

	return {
		/** The entry to mark: the first section until another is read. */
		marked(): Id | undefined {
			const ids = options.ids()
			const held = pointed() ?? reading()
			return held !== null && ids.includes(held) ? held : ids[0]
		},
		jump(id: Id, behavior: ScrollBehavior) {
			jumping = true
			options.start(id)?.scrollIntoView?.({ block: 'start', behavior })
			setReading(() => id)
		},
		/** For the page's `scroll`. */
		spy,
		/** For the page's `scrollend` and `wheel`: the reader scrolls again. */
		settle() {
			jumping = false
		},
		/** For `mouseover` on the sections. */
		point(event: MouseEvent) {
			// Between two sections, or over the page's own title: still where it
			// was. Letting go there would flash the scrolled-to section on every
			// move from one section to the next.
			const id = options.sectionOf(event.target as Element)
			if (id !== null) setPointed(() => id)
		},
		/** For `mouseleave` of the sections: the scroll answers again. */
		leave() {
			setPointed(null)
		},
	}
}
