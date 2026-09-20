dev:
	(cd app && bun run dev)

fmt: format
format:
	(cd app && bun run fmt)
	cargo fmt --all

# What CI checks, in CI's order.
check:
	cargo fmt --all --check
	cargo test --workspace
	just bindings
	git diff --exit-code app/src/ipc/bindings
	(cd app && bun run check)

# Unsigned: the .sig beside a bundle is only read by the updater, which only
# ever fetches release.yml's bundles, and signing would need that workflow's
# private key. Without this override a keyless build dies after bundling.
# A production bundle for this platform, in target/release/bundle.
build:
	(cd app && bun run build --config '{"bundle":{"createUpdaterArtifacts":false}}')

# An ordinary `cargo test` writes the bindings to a throwaway instead; see
# `.cargo/config.toml`. The path below is absolute because each crate's tests
# run with that crate as their working directory.
# Regenerates the TypeScript bindings into the repo, in the house style.
bindings:
	TS_RS_EXPORT_DIR="{{justfile_directory()}}/app/src/ipc/bindings" cargo test --workspace export_bindings
	(cd app && bunx prettier --log-level warn --write src/ipc/bindings)
