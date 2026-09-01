#!/usr/bin/env sh
# Fork-workflow setup for the submodules in external/:
#   origin       = upstream project (url in .gitmodules); default branch tracks it
#   <fork_user>  = personal fork; bare `git push` lands there via remote.pushdefault
# scripts/sparse/<name> holds sparse-checkout patterns that keep asset-heavy repos code-sized.
# Safe to re-run.
set -eu
cd "$(git rev-parse --show-toplevel)"
git submodule init
fork_user=${FORK_USER:-$(git config --global remote.pushdefault)}

for key in $(git config -f .gitmodules --name-only --get-regexp '^submodule\..*\.path$'); do
  name=${key#submodule.}; name=${name%.path}
  path=$(git config -f .gitmodules "$key")
  url=$(git config -f .gitmodules "submodule.$name.url")
  branch=$(git config -f .gitmodules "submodule.$name.branch")
  fork_url=$(printf '%s' "$url" | sed "s#github.com/[^/]*/#github.com/$fork_user/#")   # https://github.com/owner/repo -> https://github.com/fork_user/repo
  sparse="scripts/sparse/$(basename "$path")"

  [ -e "$path/.git" ] || git clone --no-checkout "$url" "$path"   # no-checkout: apply sparse patterns before the first checkout
  git -C "$path" remote add "$fork_user" "$fork_url" 2>/dev/null || true
  git -C "$path" config push.default current
  git -C "$path" config push.autosetupremote true
  if [ -f "$sparse" ]; then git -C "$path" sparse-checkout set --no-cone --stdin < "$sparse"; fi
  git -C "$path" fetch --quiet "$fork_user"
  if git -C "$path" show-ref --quiet --verify "refs/heads/$branch"; then
    git -C "$path" checkout --quiet "$branch"
  else
    git -C "$path" checkout --quiet -b "$branch" "origin/$branch"
  fi
done

git submodule absorbgitdirs
git submodule status
