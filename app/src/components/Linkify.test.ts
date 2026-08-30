import { describe, expect, test } from 'vitest'
import { split } from './Linkify'

const linked = (text: string) =>
  split(text)
    .filter((piece) => piece.href)
    .map((piece) => piece.href)

describe('finding links in what someone said', () => {
  test('a plain line has none', () => {
    expect(linked('anyone got the new sphere tweak?')).toEqual([])
    expect(split('hello').map((p) => p.text)).toEqual(['hello'])
  })

  test('a link is found among words and the words are kept', () => {
    const pieces = split('replay: https://bar-rts.com/replays/abc nice game')
    expect(pieces.map((p) => p.text).join('')).toBe(
      'replay: https://bar-rts.com/replays/abc nice game',
    )
    expect(linked('replay: https://bar-rts.com/replays/abc nice game')).toEqual(
      ['https://bar-rts.com/replays/abc'],
    )
  })

  test('trailing punctuation belongs to the sentence, not the address', () => {
    expect(linked('see https://beyondallreason.info.')).toEqual([
      'https://beyondallreason.info',
    ])
    expect(linked('(https://bar-rts.com/replays)')).toEqual([
      'https://bar-rts.com/replays',
    ])
    // …but punctuation inside a path is part of it.
    expect(linked('https://docs.google.com/a/b_c-d.e/f')).toEqual([
      'https://docs.google.com/a/b_c-d.e/f',
    ])
  })

  test('several links in one line are all found', () => {
    expect(linked('http://one.example and https://two.example/x done')).toEqual(
      ['http://one.example', 'https://two.example/x'],
    )
  })

  test('only http and https, so nothing else becomes clickable', () => {
    // A scheme the opener would refuse should not be offered in the first place.
    expect(linked('file:///C:/Windows/System32')).toEqual([])
    expect(linked('javascript:alert(1)')).toEqual([])
    expect(linked('spring://user:pw@host:8452')).toEqual([])
  })

  test('nothing is lost, whatever the line', () => {
    for (const line of [
      '',
      'https://example.com',
      'a https://example.com b https://example.org c',
      'no links at all',
    ]) {
      expect(
        split(line)
          .map((p) => p.text)
          .join(''),
      ).toBe(line)
    }
  })
})
