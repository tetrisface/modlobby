#!/usr/bin/env bun
/**
 * Prints a bun lockfile as a GitHub dependency snapshot.
 *
 * GitHub's dependency graph reads app/package.json but not app/bun.lock, so
 * Dependabot alerts on the npm packages we name and never on what they pull
 * in. Submitting the lockfile puts every installed package in the graph, and
 * Dependabot alerts on them like on any other dependency, including for
 * advisories published after the submission.
 *
 * Run from the repository root, so the manifest path is the repository's:
 *
 *   bun app/scripts/dependency-snapshot.ts app/bun.lock |
 *     gh api --method POST repos/OWNER/REPO/dependency-graph/snapshots --input -
 *
 * The commit, ref and run come from GitHub Actions' environment.
 */

type Meta = {
	dependencies?: Record<string, string>
	optionalDependencies?: Record<string, string>
	peerDependencies?: Record<string, string>
}

type Workspace = Meta & { devDependencies?: Record<string, string> }

/** The parts of bun.lock read here. An entry starts with `name@version`. */
export type Lock = {
	workspaces: Record<string, Workspace>
	packages: Record<string, [string, ...unknown[]]>
}

type Resolved = {
	package_url: string
	relationship: 'direct' | 'indirect'
	scope: 'runtime' | 'development'
	dependencies: string[]
}

export type Context = {
	manifest: string
	sha: string
	ref: string
	runId: string
	correlator: string
	scanned: string
}

const nameOf = (ident: string) => ident.slice(0, ident.lastIndexOf('@'))

/** `@scope/name@1.0.0` -> `pkg:npm/%40scope/name@1.0.0`, as the purl spec writes it. */
export const purl = (ident: string) => {
	const at = ident.lastIndexOf('@')
	return `pkg:npm/${ident.slice(0, at).replace(/^@/, '%40')}@${ident.slice(at + 1)}`
}

const metaOf = (entry: unknown[]): Meta => {
	const meta = entry[2]
	return typeof meta === 'object' && meta !== null ? meta : {}
}

/**
 * bun.lock keys a package by where it is installed: `parse5/entities` is the
 * `entities` parse5 gets instead of the hoisted one. A dependency resolves to
 * the nearest such key walking up from the dependent, as node's lookup does.
 */
export function resolve(
	lock: Lock,
	from: string,
	dep: string,
): string | undefined {
	let scope = from
	while (scope) {
		const key = `${scope}/${dep}`
		if (lock.packages[key]) return key
		const entry = lock.packages[scope]
		if (!entry) break
		scope = scope
			.slice(0, scope.length - nameOf(entry[0]).length)
			.replace(/\/$/, '')
	}
	return lock.packages[dep] ? dep : undefined
}

const depNames = (meta: Meta) => [
	...Object.keys(meta.dependencies ?? {}),
	...Object.keys(meta.optionalDependencies ?? {}),
	...Object.keys(meta.peerDependencies ?? {}),
]

/**
 * Every installed package by purl. Runtime is what the shipped app can reach
 * from `dependencies`; the rest is development. A package reachable both ways
 * is runtime, which is why the runtime roots are walked first.
 */
export function resolved(lock: Lock): Record<string, Resolved> {
	const root = lock.workspaces[''] ?? {}
	const runtimeRoots = depNames(root)
	const devRoots = Object.keys(root.devDependencies ?? {})
	const identOf = (key: string) => lock.packages[key]![0]
	const direct = new Set(
		[...runtimeRoots, ...devRoots]
			.filter((name) => lock.packages[name])
			.map((name) => purl(identOf(name))),
	)

	const out = new Map<string, Resolved>()
	const visit = (key: string, scope: Resolved['scope']) => {
		const url = purl(identOf(key))
		if (out.has(url)) return
		const children = depNames(metaOf(lock.packages[key]!)).flatMap(
			(dep) => resolve(lock, key, dep) ?? [],
		)
		out.set(url, {
			package_url: url,
			relationship: direct.has(url) ? 'direct' : 'indirect',
			scope,
			dependencies: [...new Set(children.map((child) => purl(identOf(child))))],
		})
		for (const child of children) visit(child, scope)
	}
	for (const name of runtimeRoots)
		if (lock.packages[name]) visit(name, 'runtime')
	for (const name of devRoots)
		if (lock.packages[name]) visit(name, 'development')
	return Object.fromEntries(out)
}

export const snapshot = (lock: Lock, ctx: Context) => ({
	version: 0,
	sha: ctx.sha,
	ref: ctx.ref,
	job: { correlator: ctx.correlator, id: ctx.runId },
	detector: {
		name: 'modlobby bun.lock',
		version: '1',
		url: 'https://github.com/tetrisface/modlobby/blob/main/app/scripts/dependency-snapshot.ts',
	},
	scanned: ctx.scanned,
	manifests: {
		[ctx.manifest]: {
			name: ctx.manifest,
			file: { source_location: ctx.manifest },
			resolved: resolved(lock),
		},
	},
})

if (import.meta.main) {
	const manifest = process.argv[2] ?? 'app/bun.lock'
	const lock = Bun.JSONC.parse(await Bun.file(manifest).text()) as Lock
	const env = process.env
	const ctx: Context = {
		manifest,
		sha: env.GITHUB_SHA ?? '',
		ref: env.GITHUB_REF ?? '',
		runId: env.GITHUB_RUN_ID ?? '',
		// One snapshot per manifest; a newer one with the same correlator replaces it.
		correlator: `${env.GITHUB_WORKFLOW ?? 'local'} ${manifest}`,
		scanned: new Date().toISOString(),
	}
	console.log(JSON.stringify(snapshot(lock, ctx)))
}
