#!/usr/bin/env sh
# Fork-workflow setup for the submodules in external/:
#   origin       = upstream project (url in .gitmodules); default branch tracks it
#   <fork_user>  = personal fork; bare `git push` lands there via remote.pushdefault
# Upstream repos are cloned shallow, and every fetch here keeps depth 1: a plain fetch into a
# shallow clone pulls the whole history of any fork branch that split off below the boundary.
# Nested submodules come in at depth 1 too. `git fetch --unshallow` in a submodule whose history
# you need. Repos owned by <fork_user> are cloned in full, over the ssh alias.
# scripts/sparse/<name> holds sparse-checkout patterns that keep asset-heavy repos code-sized;
# those repos are partial clones as well, so the assets the patterns leave out are never fetched.
# An existing clone keeps its shape. To re-clone one:
#   git submodule deinit -f <path> && rm -rf .git/modules/<path> && scripts/setup-submodules.sh
# Safe to re-run.
set -eu
cd "$(git rev-parse --show-toplevel)"
git submodule init

# Derived from this clone, not from machine-global config: a machine missing
# `remote.pushdefault` used to abort here and end up with a different setup.
fork_user=${FORK_USER:-$(git config --global remote.pushdefault || true)}
[ -n "$fork_user" ] || fork_user=$(git remote get-url origin | sed -E 's#.*[:/]([^/]+)/[^/]+$#\1#')
# Reuse the superproject's ssh host alias (e.g. git@github-tetrisface) so pushes to the
# forks use the right key; https fork remotes silently need a token instead.
fork_host=$(git remote get-url origin | sed -n 's#^\(git@[^:]*\):.*#\1#p')
[ -n "$fork_host" ] || fork_host=git@github.com

for key in $(git config -f .gitmodules --name-only --get-regexp '^submodule\..*\.path$'); do
  name=${key#submodule.}; name=${name%.path}
  path=$(git config -f .gitmodules "$key")
  url=$(git config -f .gitmodules "submodule.$name.url")
  branch=$(git config -f .gitmodules "submodule.$name.branch")
  fork_url="$fork_host:$fork_user/$(basename "$url" .git).git"
  sparse="scripts/sparse/$(basename "$path")"
  pin=$(git rev-parse "HEAD:$path" 2>/dev/null || true)   # commit the superproject records
  own=; case $url in */"$fork_user"/*) own=1 ;; esac

  if [ ! -e "$path/.git" ]; then
    # no-checkout: apply sparse patterns before the first checkout
    if [ -n "$own" ]; then
      git clone --no-checkout --branch "$branch" "$fork_url" "$path"
    else
      clone_filter=; if [ -f "$sparse" ]; then clone_filter=--filter=blob:none; fi
      git clone --no-checkout --depth 1 $clone_filter --branch "$branch" "$url" "$path"
    fi
    git submodule absorbgitdirs -- "$path"
  fi
  # A repo of yours may be private, and https authenticates with whatever token the credential
  # helper holds. Set in the submodule, so `git submodule sync` resetting the url cannot undo it.
  if [ -n "$own" ]; then git -C "$path" config "url.$fork_host:$fork_user/.insteadOf" "${url%/*}/"; fi

  # Fetch the way the clone was made, so a full clone is never silently truncated.
  depth=; if [ "$(git -C "$path" rev-parse --is-shallow-repository)" = true ]; then depth=--depth=1; fi
  filter=$(git -C "$path" config remote.origin.partialclonefilter || true)

  git -C "$path" remote add "$fork_user" "$fork_url" 2>/dev/null || git -C "$path" remote set-url "$fork_user" "$fork_url"
  git -C "$path" config push.default current
  git -C "$path" config push.autosetupremote true
  git -C "$path" config remote.pushdefault "$fork_user"
  # Editor autofetch runs `git fetch --all`, without --depth. Fetching origin that way only adds
  # new commits on top of the tip, but the fork would deepen, so it is fetched by name only.
  if [ -n "$depth" ]; then git -C "$path" config "remote.$fork_user.skipFetchAll" true; fi
  if [ -f "$sparse" ]; then git -C "$path" sparse-checkout set --no-cone --stdin < "$sparse"; fi
  git -C "$path" fetch --quiet $depth ${filter:+--filter=$filter} "$fork_user" || echo "  warn: cannot fetch $fork_user fork of $path"
  if git -C "$path" show-ref --quiet --verify "refs/heads/$branch"; then
    git -C "$path" checkout --quiet "$branch"
  else
    git -C "$path" checkout --quiet -b "$branch" "origin/$branch"
  fi

  # Park the branch on the pinned commit rather than the moving upstream tip. Otherwise each
  # machine lands on whatever master/main happened to be when it last ran this, the gitlink
  # shows as modified, and the two machines commit it back and forth forever. Bumping a
  # submodule stays an explicit act: git submodule update --remote <path> && git commit.
  if [ -n "$pin" ] && [ "$(git -C "$path" rev-parse HEAD)" != "$pin" ]; then
    # In a shallow clone the pin and the upstream tip are unrelated as far as git can see, so
    # commits it fetched (the shallow boundary) do not count as local ones.
    shallow="$(git -C "$path" rev-parse --absolute-git-dir)/shallow"
    [ -f "$shallow" ] || shallow=/dev/null
    if [ -n "$(git -C "$path" status --porcelain)" ]; then
      echo "  warn: $path has uncommitted changes, leaving it off the pinned commit"
    elif git -C "$path" rev-list "origin/$branch..$branch" 2>/dev/null | grep -qvxFf "$shallow"; then
      echo "  warn: $path has local commits on $branch, leaving it off the pinned commit"
    else
      git -C "$path" cat-file -e "$pin^{commit}" 2>/dev/null || git -C "$path" fetch --quiet $depth origin "$pin"
      git -C "$path" reset --hard --quiet "$pin"
    fi
  fi

  git -C "$path" submodule update --init --recursive $depth || echo "  warn: nested submodules of $path are incomplete"
done

git submodule absorbgitdirs
git submodule status
