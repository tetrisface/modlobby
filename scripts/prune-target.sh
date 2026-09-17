#!/usr/bin/env bash
#
# Prunes superseded cargo build artifacts from the target directory.
#
# Cargo names every compilation unit `<crate>-<hash>` and never removes the old
# hash once a dependency, feature or profile change produces a new one. The
# copies accumulate without bound: on this workspace `target/debug/deps` grew to
# 67 GB, 42 GB of which was units nothing refers to any more.
#
# Keeping the newest few hashes per crate leaves every artifact a current build
# reads, so a prune costs no rebuild. Only returning to a branch older than the
# kept ones recompiles, and cargo already does that on its own -- it rebuilds
# any unit whose output file has gone missing, which is why nothing here needs
# to touch `.fingerprint`.
#
# `incremental/` grows the same way. Cargo garbage-collects old sessions inside
# one `<crate>-<id>` directory, but the id follows the unit's metadata hash, and
# a directory for an id no build produces any more is never removed: 466 of
# them held 26 GB where the newest per crate held 1.4 GB. Each crate keeps a
# few more directories than it keeps hashes, since `cargo build`, `cargo test`,
# rust-analyzer and clippy each compile it under their own id.
#
# rustc also leaves scratch behind whenever a compile is killed mid-flight --
# which `tauri dev` does on every restart after a save: `*.rcgu.o` object files
# and `.tmp*`, `rustc*` and `rmeta*` directories in `deps/`. A finished compile
# deletes its own, so anything older than an hour belongs to no one.
set -euo pipefail

keep=3
keep_incremental=5
dry_run=0
# Anchored to the script's own location, not the caller's: `bun run` invokes
# this from `app/`, where a relative `target` would find the empty `app/target`
# rather than the workspace one at the root.
target_dir=${CARGO_TARGET_DIR:-$(cd "$(dirname "$0")/.." && pwd)/target}

usage() {
	cat <<'USAGE'
usage: prune-target.sh [-k N] [-i N] [-n] [-t DIR]

  -k, --keep N              hashes to keep per crate (default 3)
  -i, --keep-incremental N  incremental directories to keep per crate (default 5)
  -n, --dry-run             report what would be removed, remove nothing
  -t, --target DIR          target directory (default $CARGO_TARGET_DIR, else the
                            workspace target beside this script, whatever the cwd)
USAGE
}

while [ $# -gt 0 ]; do
	case $1 in
	-k | --keep) keep=$2 && shift 2 ;;
	-i | --keep-incremental) keep_incremental=$2 && shift 2 ;;
	-n | --dry-run) dry_run=1 && shift ;;
	-t | --target) target_dir=$2 && shift 2 ;;
	-h | --help) usage && exit 0 ;;
	*) echo "prune-target: unknown argument: $1" >&2 && usage >&2 && exit 2 ;;
	esac
done

# Nothing built yet is nothing to prune, not a failure -- `just dev` runs this
# on a fresh clone too.
[ -d "$target_dir" ] || { echo "prune-target: no target directory at $target_dir, nothing to do" && exit 0; }
find . -maxdepth 0 -printf '' 2>/dev/null || { echo "prune-target: needs GNU find (-printf)" >&2 && exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Reads `unit <TAB> mtime <TAB> crate <TAB> path` lines, ranks each crate's
# units by how recently anything in them was written, and prints the paths of
# every unit past the newest `$1`.
doom() {
	local entries
	entries=$(mktemp -p "$work")
	cat >"$entries"
	awk -F'\t' -v OFS='\t' '
		{ if ($2 > mtime[$1]) mtime[$1] = $2; crate[$1] = $3 }
		END { for (unit in mtime) print crate[unit], mtime[unit], unit }
	' "$entries" |
		sort -t"$(printf '\t')" -k1,1 -k2,2nr |
		awk -F'\t' -v keep="$1" '
			$1 != prev { prev = $1; n = 0 }
			{ n++; if (n > keep) print $3 }
		' >"$entries.doomed"
	awk -F'\t' 'NR == FNR { doomed[$0] = 1; next } $1 in doomed { print $4 }' \
		"$entries.doomed" "$entries"
}

# Every hashed entry, tagged with the unit it belongs to. A unit is one
# (directory, crate, hash) triple: the rlib, rmeta, pdb, object files and dep
# list cargo emitted together all carry the same hash and live or die together.
for dir in "$target_dir"/*/deps "$target_dir"/*/examples "$target_dir"/*/build; do
	[ -d "$dir" ] || continue
	find "$dir" -mindepth 1 -maxdepth 1 -printf '%T@\t%f\t%p\n'
done | awk -F'\t' -v OFS='\t' '
	{
		name = $2
		if (!match(name, /-[0-9a-f]{16}([.-]|$)/)) next
		crate = substr(name, 1, RSTART - 1)
		hash = substr(name, RSTART + 1, 16)
		sub(/^lib/, "", crate)
		path = $3
		dir = path
		sub(/\/[^\/]*$/, "", dir)
		print dir "|" crate "|" hash, int($1), dir "|" crate, path
	}
' | doom "$keep" >"$work/doomed"

# Each `<crate>-<id>` directory is one unit. Its sessions are rewritten on every
# compile while the directory itself may not be, so the newest write anywhere in
# its top two levels dates it.
for dir in "$target_dir"/*/incremental; do
	[ -d "$dir" ] || continue
	find "$dir" -mindepth 1 -maxdepth 2 -printf '%T@\t%P\t%h\n'
done | awk -F'\t' -v OFS='\t' '
	{
		unit = $2
		sub(/\/.*$/, "", unit)
		root = $3
		if ($2 ~ /\//) sub(/\/[^\/]*$/, "", root)
		crate = unit
		sub(/-[0-9a-z]+$/, "", crate)
		print root "/" unit, int($1), root "|" crate, root "/" unit
	}
' | doom "$keep_incremental" >>"$work/doomed"

for dir in "$target_dir"/*/deps "$target_dir"/*/examples; do
	[ -d "$dir" ] || continue
	find "$dir" -mindepth 1 -maxdepth 1 -mmin +60 \
		\( \( -type f -name '*.rcgu.o' \) -o \( -type d \( -name '.tmp*' -o -name 'rustc*' -o -name 'rmeta*' \) \) \) \
		-print
done >>"$work/doomed"

sort -u -o "$work/doomed" "$work/doomed"

count=$(wc -l <"$work/doomed" | tr -d ' ')
[ "$count" -gt 0 ] || { echo "prune-target: nothing superseded, $target_dir is already lean" && exit 0; }

bytes=$(du -scb --files0-from=<(tr '\n' '\0' <"$work/doomed") 2>/dev/null | tail -1 | cut -f1)
printf 'prune-target: %s entries, %.1f GB, keeping the %d newest hashes and %d incremental directories per crate\n' \
	"$count" "$(echo "$bytes" | awk '{ print $1 / 1073741824 }')" "$keep" "$keep_incremental"

[ "$dry_run" -eq 0 ] || { echo "prune-target: dry run, nothing removed" && exit 0; }

tr '\n' '\0' <"$work/doomed" | xargs -0 rm -rf --
echo "prune-target: removed"
