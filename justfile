dev:
	(cd app && bun run dev)
format:
	prettier --write app
	cargo fmt -all
