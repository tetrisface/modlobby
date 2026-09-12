#!/usr/bin/env sh
# Fork-workflow setup for the submodules in external/:
#   origin       = upstream project (url in .gitmodules); default branch tracks it
#   <fork_user>  = personal fork; bare `git push` lands there via remote.pushdefault
# Upstream repos are blobless partial clones (--filter=blob:none): the whole commit graph, with
# file contents fetched only when a checkout needs them. They used to be depth-1 shallow, which
# left the pinned commit and origin/<branch> with no common ancestor, so git called every update
# a divergence and `git pull` -- the editor's Sync button -- refused to run. A clone left over
# from that is unshallowed on the next run. Repos owned by <fork_user> are cloned in full, over
# the ssh alias.
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
      git clone --no-checkout --filter=blob:none --branch "$branch" "$url" "$path"
    fi
    git submodule absorbgitdirs -- "$path"
  fi
  # A repo of yours may be private, and https authenticates with whatever token the credential
  # helper holds. Set in the submodule, so `git submodule sync` resetting the url cannot undo it.
  if [ -n "$own" ]; then git -C "$path" config "url.$fork_host:$fork_user/.insteadOf" "${url%/*}/"; fi

  # Heal a clone made by the old depth-1 version of this script. Blobless, so it costs commits
  # and trees only, and it is what makes `git pull` a plain fast-forward again.
  if [ "$(git -C "$path" rev-parse --is-shallow-repository)" = true ]; then
    git -C "$path" fetch --quiet --unshallow --filter=blob:none origin ||
      echo "  warn: could not unshallow $path; pulls there will still report divergent branches"
  fi
  filter=$(git -C "$path" config remote.origin.partialclonefilter || true)

  git -C "$path" remote add "$fork_user" "$fork_url" 2>/dev/null || git -C "$path" remote set-url "$fork_user" "$fork_url"
  git -C "$path" config push.default current
  git -C "$path" config push.autosetupremote true
  git -C "$path" config remote.pushdefault "$fork_user"
  # These track upstream and carry no local work, so an update is always a fast-forward. Say so:
  # a pull that cannot fast-forward is a real problem here, not something to paper over with a
  # merge commit the fork would then carry forever.
  git -C "$path" config pull.ff only
  if [ -f "$sparse" ]; then git -C "$path" sparse-checkout set --no-cone --stdin < "$sparse"; fi
  git -C "$path" fetch --quiet ${filter:+--filter=$filter} "$fork_user" || echo "  warn: cannot fetch $fork_user fork of $path"
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
    if [ -n "$(git -C "$path" status --porcelain)" ]; then
      echo "  warn: $path has uncommitted changes, leaving it off the pinned commit"
    elif [ -n "$(git -C "$path" rev-list "origin/$branch..$branch" 2>/dev/null)" ]; then
      echo "  warn: $path has local commits on $branch, leaving it off the pinned commit"
    else
      git -C "$path" cat-file -e "$pin^{commit}" 2>/dev/null || git -C "$path" fetch --quiet origin "$pin"
      git -C "$path" reset --hard --quiet "$pin"
    fi
  fi

  git -C "$path" submodule update --init --recursive || echo "  warn: nested submodules of $path are incomplete"
done

git submodule absorbgitdirs
git submodule status
