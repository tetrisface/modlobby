import { createRoot, createSignal } from 'solid-js'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { STEP, createDrawnSize, step } from './drawn'

/** A ResizeObserver the test drives by hand. */
class FakeObserver {
  static latest: FakeObserver | undefined
  static observed: HTMLElement[] = []
  disconnected = false
  constructor(private readonly callback: () => void) {
    FakeObserver.latest = this
  }
  observe(element: HTMLElement) {
    FakeObserver.observed.push(element)
  }
  disconnect() {
    this.disconnected = true
  }
  fire() {
    this.callback()
  }
}

/** Only what is measured: an element with a size the test can change. */
function box(width: number, height: number) {
  const element = {
    clientWidth: width,
    clientHeight: height,
    resize(w: number, h: number) {
      element.clientWidth = w
      element.clientHeight = h
    },
  }
  return element
}

const asElement = (b: ReturnType<typeof box>) => b as unknown as HTMLElement

afterEach(() => {
  vi.unstubAllGlobals()
  FakeObserver.observed = []
})

describe('a side in steps', () => {
  it('rounds up to whole steps and leaves zero alone', () => {
    expect(step(0)).toBe(0)
    expect(step(1)).toBe(STEP)
    expect(step(STEP)).toBe(STEP)
    expect(step(STEP + 1)).toBe(2 * STEP)
  })
})

describe('the size a box is drawn', () => {
  it('is nothing until the element exists and has a size', () => {
    vi.stubGlobal('ResizeObserver', FakeObserver)
    const [el, setEl] = createSignal<HTMLElement>()
    const { drawn, dispose } = createRoot((dispose) => ({
      drawn: createDrawnSize(el),
      dispose,
    }))
    expect(drawn()).toBeNull()

    const unlaid = box(0, 0)
    setEl(asElement(unlaid))
    expect(drawn()).toBeNull()

    unlaid.resize(300, 300)
    FakeObserver.latest?.fire()
    expect(drawn()).toEqual({ width: 5 * STEP, height: 5 * STEP })
    dispose()
  })

  it('reports the same size once per step, and stops watching when disposed', () => {
    vi.stubGlobal('ResizeObserver', FakeObserver)
    const element = box(100, 100)
    const { drawn, dispose } = createRoot((dispose) => ({
      drawn: createDrawnSize(() => asElement(element)),
      dispose,
    }))
    const first = drawn()
    expect(first).toEqual({ width: 2 * STEP, height: 2 * STEP })
    expect(FakeObserver.observed).toEqual([asElement(element)])

    element.resize(120, 120)
    FakeObserver.latest?.fire()
    expect(drawn()).toBe(first)

    const observer = FakeObserver.latest
    dispose()
    expect(observer?.disconnected).toBe(true)
  })
})
