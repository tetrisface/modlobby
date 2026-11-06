import { describe, expect, test } from 'vitest'
import { dropModel, modelFor, seedModel } from './monaco'

describe('a document model', () => {
	test('started empty, takes a laid-out tweak back exactly as given', () => {
		// The slot's model is made before anything is written in it. Seeded
		// with the room's text, it must hand that same text back: the store
		// compares the two to say whether the slot holds an unsent edit.
		const model = modelFor('slot:test-empty', '', 'lua')
		const text = '-- Name\ndo\n\tlocal x = 1\nend\n'
		seedModel(model, text)
		expect(model.getValue()).toBe(text)
		dropModel('slot:test-empty')
	})

	test('made from Windows line endings, holds the editor’s own', () => {
		const model = modelFor('draft:test-crlf', '-- Name\r\nreturn {}\r\n', 'lua')
		expect(model.getValue()).toBe('-- Name\nreturn {}\n')
		dropModel('draft:test-crlf')
	})
})
