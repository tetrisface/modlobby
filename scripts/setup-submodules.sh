#!/usr/bin/env sh
# Fork-workflow setup for the submodules in external/:
#   origin       = upstream project (url in .gitmodules); default branch tracks it
#   <fork_user>  = personal fork; bare `git push` lands there via remote.pushdefault
# scripts/sparse/<name> holds sparse-checkout patterns that keep asset-heavy repos code-sized.
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

  [ -e "$path/.git" ] || git clone --no-checkout "$url" "$path"   # no-checkout: apply sparse patterns before the first checkout
  git -C "$path" remote add "$fork_user" "$fork_url" 2>/dev/null || git -C "$path" remote set-url "$fork_user" "$fork_url"
  git -C "$path" config push.default current
  git -C "$path" config push.autosetupremote true
  git -C "$path" config remote.pushdefault "$fork_user"
  if [ -f "$sparse" ]; then git -C "$path" sparse-checkout set --no-cone --stdin < "$sparse"; fi
  git -C "$path" fetch --quiet "$fork_user" || echo "  warn: cannot fetch $fork_user fork of $path"
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
    elif [ -n "$(git -C "$path" log --oneline "origin/$branch..$branch" 2>/dev/null)" ]; then
      echo "  warn: $path has local commits on $branch, leaving it off the pinned commit"
    else
      git -C "$path" cat-file -e "$pin^{commit}" 2>/dev/null || git -C "$path" fetch --quiet origin
      git -C "$path" reset --hard --quiet "$pin"
    fi
  fi
done

git submodule absorbgitdirs
git submodule status
