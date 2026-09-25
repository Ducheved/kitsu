#!/usr/bin/env bash
# Versions and the changelog are computed from commit messages, so a pull
# request's title and every commit it adds must be a Conventional Commit
# (https://www.conventionalcommits.org/en/v1.0.0/):
#
#   <type>[(scope)][!]: <description>
#
# Usage: conventional.sh <base-sha> <head-sha>   (PR_TITLE in the environment)
# Merge commits are skipped; git's own `Revert "..."` subject is accepted.
set -euo pipefail

types='build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test'
pattern="^(($types)(\\([A-Za-z0-9._/-]+\\))?!?: [^[:space:]].*|Revert \".+\")\$"

bad=0
report() {
  # Never print untrusted text at the start of a line: a line starting with
  # `::` is a workflow command.
  echo "::error title=Not a Conventional Commit::$1"
  printf '    %s\n' "$2"
  bad=1
}

if [[ -n "${PR_TITLE:-}" ]]; then
  if [[ "$PR_TITLE" =~ $pattern ]]; then
    printf 'ok  title: %s\n' "$PR_TITLE"
  else
    report "pull request title" "$PR_TITLE"
  fi
fi

if [[ $# -eq 2 ]]; then
  while IFS= read -r line; do
    sha=${line%% *}
    subject=${line#* }
    if [[ "$subject" =~ $pattern ]]; then
      printf 'ok  %s %s\n' "${sha:0:7}" "$subject"
    else
      report "commit ${sha:0:7}" "$subject"
    fi
  done < <(git log --no-merges --format='%H %s' "$1..$2")
fi

if [[ $bad -ne 0 ]]; then
  echo
  echo "Expected '<type>[(scope)][!]: <description>' with type one of: ${types//|/, }."
  echo "Breaking changes: 'feat!: ...' or a 'BREAKING CHANGE: ...' footer."
  echo "Reword with 'git rebase -i' (commits) or edit the pull request title."
fi
exit "$bad"
