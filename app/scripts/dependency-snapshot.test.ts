import { describe, expect, it } from 'vitest'
import { type Lock, purl, resolve, resolved } from './dependency-snapshot'

// app depends on `ui`; `ui` and the dev tool `build` both use `util`, but
// `build` has its own older copy installed under it.
const lock: Lock = {
	workspaces: {
		'': {
			dependencies: { ui: '1.0.0' },
			devDependencies: { build: '2.0.0' },
		},
	},
	packages: {
		ui: [
			'ui@1.0.0',
			'',
			{ dependencies: { util: '^3.0.0', '@scope/icons': '1.0.0' } },
			'sha512-',
		],
		'@scope/icons': ['@scope/icons@1.0.0', '', {}, 'sha512-'],
		util: ['util@3.1.0', '', { peerDependencies: { absent: '*' } }, 'sha512-'],
		build: [
			'build@2.0.0',
			'',
			{ dependencies: { util: '^2.0.0', ui: '1.0.0' } },
			'sha512-',
		],
		'build/util': ['util@2.4.0', '', {}, 'sha512-'],
	},
}

describe('purl', () => {
	it('percent-encodes the scope', () => {
		expect(purl('@scope/icons@1.0.0')).toBe('pkg:npm/%40scope/icons@1.0.0')
		expect(purl('util@3.1.0')).toBe('pkg:npm/util@3.1.0')
	})
})

describe('resolve', () => {
	it('prefers the copy installed under the dependent', () => {
		expect(resolve(lock, 'build', 'util')).toBe('build/util')
		expect(resolve(lock, 'ui', 'util')).toBe('util')
	})

	it('walks up past a scoped parent', () => {
		const nested: Lock = {
			workspaces: { '': {} },
			packages: {
				x: ['x@1.0.0', '', {}, ''],
				'x/@s/y': ['@s/y@1.0.0', '', {}, ''],
				'x/@s/y/z': ['z@1.0.0', '', {}, ''],
				w: ['w@1.0.0', '', {}, ''],
			},
		}
		expect(resolve(nested, 'x/@s/y', 'z')).toBe('x/@s/y/z')
		expect(resolve(nested, 'x/@s/y/z', 'w')).toBe('w')
	})

	it('drops what is not installed', () => {
		expect(resolve(lock, 'util', 'absent')).toBeUndefined()
	})
})

describe('resolved', () => {
	const graph = resolved(lock)

	it('lists every installed package once', () => {
		expect(Object.keys(graph).sort()).toEqual([
			'pkg:npm/%40scope/icons@1.0.0',
			'pkg:npm/build@2.0.0',
			'pkg:npm/ui@1.0.0',
			'pkg:npm/util@2.4.0',
			'pkg:npm/util@3.1.0',
		])
	})

	it('scopes by what the shipped app reaches', () => {
		expect(graph['pkg:npm/util@3.1.0']!.scope).toBe('runtime')
		expect(graph['pkg:npm/util@2.4.0']!.scope).toBe('development')
		expect(graph['pkg:npm/build@2.0.0']!.scope).toBe('development')
	})

	it('marks only the manifest names as direct', () => {
		expect(graph['pkg:npm/ui@1.0.0']!.relationship).toBe('direct')
		expect(graph['pkg:npm/build@2.0.0']!.relationship).toBe('direct')
		expect(graph['pkg:npm/util@3.1.0']!.relationship).toBe('indirect')
	})

	it('links each package to the copies it resolves', () => {
		expect(graph['pkg:npm/build@2.0.0']!.dependencies).toEqual([
			'pkg:npm/util@2.4.0',
			'pkg:npm/ui@1.0.0',
		])
	})
})
