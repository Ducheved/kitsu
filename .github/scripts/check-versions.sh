#!/usr/bin/env bash
# Kitsu has one version. Fail if any place that states it disagrees with
# crates/kitsu/Cargo.toml. release-please bumps all of them in the release
# PR; Cargo.lock is then updated by `cargo update --workspace`.
set -euo pipefail

toml_version() {
  sed -n 's/^version = "\(.*\)"$/\1/p' "$1" | head -n 1
}
lock_version() {
  awk -v name="$1" '$0 == "name = \"" name "\"" { getline; gsub(/^version = "|"$/, ""); print; exit }' Cargo.lock
}

want=$(toml_version crates/kitsu/Cargo.toml)
status=0
check() {
  local file=$1 what=$2 got=$3
  if [[ "$got" != "$want" ]]; then
    echo "::error file=$file::$what is '$got', crates/kitsu/Cargo.toml says '$want'"
    status=1
  else
    echo "ok  $file $what $got"
  fi
}

check app/src-tauri/Cargo.toml package.version "$(toml_version app/src-tauri/Cargo.toml)"
check app/package.json version "$(jq -r .version app/package.json)"
check app/package-lock.json version "$(jq -r .version app/package-lock.json)"
check app/package-lock.json 'packages."".version' "$(jq -r '.packages[""].version' app/package-lock.json)"
check app/src-tauri/tauri.conf.json version "$(jq -r .version app/src-tauri/tauri.conf.json)"
check .release-please-manifest.json '"."' "$(jq -r '."."' .release-please-manifest.json)"
check Cargo.lock 'kitsu version' "$(lock_version kitsu)"
check Cargo.lock 'kitsu-app version' "$(lock_version kitsu-app)"
exit "$status"
