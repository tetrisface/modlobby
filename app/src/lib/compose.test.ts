import { describe, expect, test } from 'vitest'
import {
  HISTORY_MAX,
  complete,
  completions,
  recall,
  remember,
  wordAt,
} from './compose'

describe('remembering what was sent', () => {
  test('lines are kept in the order they were sent', () => {
    let history: string[] = []
    history = remember(history, '!balance')
    history = remember(history, 'gg')
    expect(history).toEqual(['!balance', 'gg'])
  })

  test('blank lines and immediate repeats are not worth keeping', () => {
    expect(remember(['gg'], '   ')).toEqual(['gg'])
    expect(remember(['gg'], 'gg')).toEqual(['gg'])
    // ...but the same line again later is a real recall target.
    expect(remember(['gg', 'hi'], 'gg')).toEqual(['gg', 'hi', 'gg'])
  })

  test('the oldest fall off once there are too many', () => {
    let history: string[] = []
    for (let n = 0; n < HISTORY_MAX + 10; n += 1)
      history = remember(history, `line ${n}`)
    expect(history.length).toBe(HISTORY_MAX)
    expect(history[0]).toBe('line 10')
  })
})

describe('walking back through it', () => {
  const history = ['first', 'second', 'third']

  test('up walks back from the newest', () => {
    let step = recall(history, -1, 1, 'draft')
    expect(step).toEqual({ at: 0, text: 'third' })
    step = recall(history, step.at, 1, 'draft')
    expect(step).toEqual({ at: 1, text: 'second' })
    step = recall(history, step.at, 1, 'draft')
    expect(step).toEqual({ at: 2, text: 'first' })
  })

  test('the oldest is where it stops', () => {
    expect(recall(history, 2, 1, 'draft')).toEqual({ at: 2, text: 'first' })
  })

  test('down comes back, and past the newest is what you were typing', () => {
    expect(recall(history, 1, -1, 'draft')).toEqual({ at: 0, text: 'third' })
    expect(recall(history, 0, -1, 'draft')).toEqual({ at: -1, text: 'draft' })
  })

  test('an empty history leaves the draft alone', () => {
    expect(recall([], -1, 1, 'draft')).toEqual({ at: -1, text: 'draft' })
  })
})

describe('finishing a name', () => {
  const names = ['Skywalker', 'BlueSky', 'sky_bot', '[Crd]XxStormKittyxX']

  test('a prefix match is offered before a mere containment', () => {
    expect(completions('sky', names)).toEqual([
      'sky_bot',
      'Skywalker',
      'BlueSky',
    ])
  })

  test('case does not matter, since nobody types a name as registered', () => {
    expect(completions('SKYW', names)).toEqual(['Skywalker'])
    expect(completions('xxstorm', names)).toEqual(['[Crd]XxStormKittyxX'])
  })

  test('nothing to go on offers nothing', () => {
    expect(completions('', names)).toEqual([])
    expect(completions('zzz', names)).toEqual([])
  })

  test('a name already whole is not offered as its own completion', () => {
    expect(completions('skywalker', names)).toEqual([])
  })
})

describe('putting the name into the line', () => {
  test('the word under the caret is what gets replaced', () => {
    expect(wordAt('hey sky', 7)).toEqual({ word: 'sky', from: 4 })
    expect(wordAt('sky', 3)).toEqual({ word: 'sky', from: 0 })
  })

  test('at the start of a line, a name is being addressed', () => {
    expect(complete('sky', 3, 'Skywalker')).toEqual({
      text: 'Skywalker: ',
      caret: 11,
    })
  })

  test('anywhere else it is just a name in a sentence', () => {
    expect(complete('nice one sky', 12, 'Skywalker')).toEqual({
      text: 'nice one Skywalker ',
      caret: 19,
    })
  })

  test('whatever follows the caret is kept', () => {
    expect(complete('sky and rest', 3, 'Skywalker')).toEqual({
      text: 'Skywalker:  and rest',
      caret: 11,
    })
  })
})
