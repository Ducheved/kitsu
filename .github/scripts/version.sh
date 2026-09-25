#!/usr/bin/env bash
# The version this checkout builds as. Run in CI from the repository root
# with full history (actions/checkout `fetch-depth: 0`).
#
#   release=true   HEAD carries the tag v<Cargo version>
#     version=0.2.0
#   release=false  anything else: the next patch after the Cargo version,
#                  pre-release dev.<commits since the last v* tag> (all
#                  commits if there is none yet), build metadata g<sha>.
#                  SemVer orders it after the last release and before the
#                  next one, whatever that turns out to be.
#     version=0.2.1-dev.14+g1a2b3c4
#   bundle=0.2.0   what installers carry: always the plain Cargo version,
#                  because MSI accepts only numeric versions.
#
# Prints key=value lines; with GITHUB_OUTPUT set, appends them there too.
set -euo pipefail

cargo_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' crates/kitsu/Cargo.toml | head -n 1)
if [[ -z "$cargo_version" ]]; then
  echo "version.sh: no version in crates/kitsu/Cargo.toml" >&2
  exit 1
fi
if [[ "$(git rev-parse --is-shallow-repository)" == "true" ]]; then
  echo "version.sh: shallow clone, commit count would be wrong; check out with fetch-depth: 0" >&2
  exit 1
fi

sha=$(git rev-parse --short=7 HEAD)
if git tag --points-at HEAD | grep -qxF "v$cargo_version"; then
  release=true
  version=$cargo_version
else
  release=false
  if last=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' HEAD 2>/dev/null); then
    count=$(git rev-list --count "$last..HEAD")
  else
    count=$(git rev-list --count HEAD)
  fi
  core=${cargo_version%%+*}
  if [[ "$core" == *-* ]]; then
    # Already a pre-release (1.0.0-rc.1): 1.0.0-rc.1.dev.N sorts after it.
    base="$core.dev.$count"
  else
    IFS=. read -r major minor patch <<<"$core"
    base="$major.$minor.$((patch + 1))-dev.$count"
  fi
  version="$base+g$sha"
fi

out="release=$release
version=$version
bundle=$cargo_version
sha=$sha"
echo "$out"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "$out" >>"$GITHUB_OUTPUT"
fi
