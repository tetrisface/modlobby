dev:
	(cd app && bun run dev)
format:
	(cd app && bun run fmt)
	cargo fmt --all
# What CI checks, in CI's order -- prettier after `cargo test`, which regenerates the bindings.
check:
	cargo fmt --all --check
	cargo test --workspace
	(cd app && bunx prettier --log-level warn --write src/ipc/bindings && git diff --exit-code src/ipc/bindings)
	(cd app && bun run check)
