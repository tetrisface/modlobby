dev:
	(cd app && bun run dev)

fmt: format
format:
	(cd app && bun run fmt)
	cargo fmt --all

# Regenerates the TypeScript bindings into the repo and puts them in the house
# style. An ordinary `cargo test` writes them to a throwaway instead; see
# `.cargo/config.toml`.
bindings:
	TS_RS_EXPORT_DIR=app/src/ipc/bindings cargo test --workspace export_bindings
	(cd app && bunx prettier --log-level warn --write src/ipc/bindings)

# What CI checks, in CI's order.
check:
	cargo fmt --all --check
	cargo test --workspace
	just bindings
	git diff --exit-code app/src/ipc/bindings
	(cd app && bun run check)
