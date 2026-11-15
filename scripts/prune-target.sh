#!/usr/bin/env bash
#
# Prunes superseded cargo build artifacts from the target directory.
#
# Cargo gives every compilation unit a directory of its own,
# `<profile>/build/<package>/<hash>/`: the fingerprint in `fingerprint/` and
# rustc's outputs in `out/` -- or, for a build script's run, its OUT_DIR there
# and its output log in `run/`. Any dependency, feature, profile or toolchain
# change makes a new hash, and cargo never removes the unit it replaced. The
# copies accumulate without bound: in the layout before Cargo 1.100, which kept
# the same files together in `deps/`, that directory grew to 67 GB on this
# workspace, 42 GB of which was units nothing referred to any more.
#
# A fingerprint's file name says which target the unit builds
# (`lib-<crate>.json`, `test-lib-<crate>.json`,
# `run-build-script-build-script-build.json`), and its `path` hashes the source
# path, which carries a registry crate's version. The units of one target of
# one version are its generations, and keeping the newest few keeps what
# current builds read, so a prune costs no rebuild. Ranking a package's units
# all together would not: one build compiles windows-sys 0.59, 0.60 and 0.61,
# two of those twice with different profiles. Only returning to a branch older
# than the kept generations recompiles, and cargo already does that on its own
# -- it rebuilds any unit whose directory has gone missing.
#
# `incremental/` grows the same way. Cargo garbage-collects old sessions inside
# one `<crate>-<id>` directory, but the id follows the unit's metadata hash, and
# a directory for an id no build produces any more is never removed: 466 of
# them held 26 GB where the newest per crate held 1.4 GB. Each crate keeps a
# few more directories than a target keeps generations, since `cargo build`,
# `cargo test`, rust-analyzer and clippy each compile it under their own id.
#
# rustc also leaves scratch behind whenever a compile is killed mid-flight --
# which `tauri dev` does on every restart after a save: `*.rcgu.o` object files
# and `.tmp*`, `rustc*` and `rmeta*` directories in a unit's `out/`. A finished
# compile deletes its own, so anything older than an hour belongs to no one.
#
# Cargo before 1.100 sorted the same files by kind instead: `deps/`,
# `.fingerprint/`, `build/<package>-<hash>/` and hashed copies in `examples/`.
# Nothing reads those any more, so they go whole.
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

  -k, --keep N              generations to keep of each target (default 3)
  -i, --keep-incremental N  incremental directories to keep per crate (default 5)
  -n, --dry-run             report what would be removed and its size, remove nothing
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
# macOS ships BSD find/du, which have neither -printf nor --files0-from; homebrew's
# GNU ones are prefixed with g.
find=$(command -v gfind || command -v find)
du=$(command -v gdu || command -v du)
"$find" . -maxdepth 0 -printf "" 2>/dev/null || { echo "prune-target: needs GNU find (-printf): brew install findutils" >&2 && exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Reads `unit <TAB> mtime <TAB> group <TAB> path` lines, ranks each group's
# units by how recently anything in them was written, and prints the paths of
# every unit past the newest `$1`.
doom() {
	local entries
	entries=$(mktemp "$work/XXXXXXXX")
	cat >"$entries"
	awk -F'\t' -v OFS='\t' '
		{ if ($2 > mtime[$1]) mtime[$1] = $2; group[$1] = $3 }
		END { for (unit in mtime) print group[unit], mtime[unit], unit }
	' "$entries" |
		sort -t"$(printf '\t')" -k1,1 -k2,2nr |
		awk -F'\t' -v keep="$1" '
			$1 != prev { prev = $1; n = 0 }
			{ n++; if (n > keep) print $3 }
		' >"$entries.doomed"
	awk -F'\t' 'NR == FNR { doomed[$0] = 1; next } $1 in doomed { print $4 }' \
		"$entries.doomed" "$entries"
}

# Every unit, grouped by package, target and version. Only a compile writes a
# fingerprint, so its time is when the unit was last built.
for dir in "$target_dir"/*/build; do
	[ -d "$dir" ] || continue
	"$find" "$dir" -mindepth 4 -maxdepth 4 -path '*/fingerprint/*.json' -printf '%T@\t%p\n'
done | awk -F'\t' -v OFS='\t' '
	{
		fingerprint = $2
		version = ""
		if ((getline json <fingerprint) > 0 && match(json, /"path":[0-9]+/))
			version = substr(json, RSTART + 7, RLENGTH - 7)
		close(fingerprint)
		unit = fingerprint
		sub(/\/fingerprint\/[^\/]*$/, "", unit)
		package = unit
		sub(/\/[^\/]*$/, "", package)
		target = fingerprint
		sub(/^.*\//, "", target)
		print unit, int($1), package "|" target "|" version, unit
	}
' | doom "$keep" >"$work/doomed"

# Each `<crate>-<id>` directory is one unit. Its sessions are rewritten on every
# compile while the directory itself may not be, so the newest write anywhere in
# its top two levels dates it.
for dir in "$target_dir"/*/incremental; do
	[ -d "$dir" ] || continue
	"$find" "$dir" -mindepth 1 -maxdepth 2 -printf '%T@\t%P\t%h\n'
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

# The `out/` beside a `run/` is a build script's OUT_DIR, not rustc's, and is
# left alone.
for dir in "$target_dir"/*/build; do
	[ -d "$dir" ] || continue
	"$find" "$dir" -mindepth 4 -maxdepth 4 -path '*/out/*' -mmin +60 \
		\( \( -type f -name '*.rcgu.o' \) -o \( -type d \( -name '.tmp*' -o -name 'rustc*' -o -name 'rmeta*' \) \) \) \
		-print
done | while IFS= read -r path; do
	[ -d "${path%/out/*}/run" ] || printf '%s\n' "$path"
done >>"$work/doomed"

# The layout before Cargo 1.100.
for dir in "$target_dir"/*/deps "$target_dir"/*/.fingerprint; do
	[ -d "$dir" ] || continue
	printf '%s\n' "$dir"
done >>"$work/doomed"
for dir in "$target_dir"/*/build "$target_dir"/*/examples; do
	[ -d "$dir" ] || continue
	"$find" "$dir" -mindepth 1 -maxdepth 1 -printf '%f\t%p\n'
done | awk -F'\t' '$1 ~ /-[0-9a-f]{16}([.-]|$)/ { print $2 }' >>"$work/doomed"

sort -u -o "$work/doomed" "$work/doomed"

count=$(wc -l <"$work/doomed" | tr -d ' ')
[ "$count" -gt 0 ] || { echo "prune-target: nothing superseded, $target_dir is already lean" && exit 0; }

printf 'prune-target: %s entries, keeping the %d newest generations of each target and %d incremental directories per crate\n' \
	"$count" "$keep" "$keep_incremental"

# Only a dry run measures: `du` stats every file under the doomed directories,
# which on Windows takes longer than deleting them.
if [ "$dry_run" -eq 1 ]; then
	bytes=$("$du" -scb --files0-from=<(tr '\n' '\0' <"$work/doomed") 2>/dev/null | tail -1 | cut -f1)
	printf 'prune-target: dry run, would free %.1f GB, nothing removed\n' "$(echo "$bytes" | awk '{ print $1 / 1073741824 }')"
	exit 0
fi

tr '\n' '\0' <"$work/doomed" | xargs -0 rm -rf --
echo "prune-target: removed"
