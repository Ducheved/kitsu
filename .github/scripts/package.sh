#!/usr/bin/env bash
# Collect what a build produced into dist/<kind>/, ready to upload.
#
#   package.sh cli <rust-target> <version>
#       target/<rust-target>/release/{kitsu,kitsu-test-agent} + LICENSE +
#       README.md -> dist/cli/kitsu-<version>-<rust-target>.{tar.gz,zip}
#       Checks that `kitsu --version` says <version> when the binary can
#       run here.
#   package.sh app <rust-target>
#       installers from target/<rust-target>/release/bundle -> dist/app/
set -euo pipefail
shopt -s nullglob

kind=$1
target=$2
release_dir="target/$target/release"

case "$kind" in
cli)
  version=$3
  exe=""
  if [[ "$target" == *windows* ]]; then exe=".exe"; fi
  name="kitsu-$version-$target"
  stage="dist/stage/$name"
  rm -rf "$stage"
  mkdir -p "$stage" dist/cli
  cp "$release_dir/kitsu$exe" "$release_dir/kitsu-test-agent$exe" LICENSE README.md "$stage/"

  host=$(rustc -vV | sed -n 's/^host: //p')
  if [[ "$target" == "$host" ]]; then
    got=$("$stage/kitsu$exe" --version | tr -d '\r')
    if [[ "$got" != "kitsu $version" ]]; then
      echo "::error::kitsu --version printed '$got', expected 'kitsu $version'"
      exit 1
    fi
    echo "kitsu --version: $got"
  fi

  if [[ -n "$exe" ]]; then
    (cd dist/stage && 7z a -tzip -bso0 "../cli/$name.zip" "$name")
  else
    tar -C dist/stage -czf "dist/cli/$name.tar.gz" "$name"
  fi
  rm -rf dist/stage
  ;;
app)
  mkdir -p dist/app
  bundle="$release_dir/bundle"
  found=("$bundle"/deb/*.deb "$bundle"/rpm/*.rpm "$bundle"/appimage/*.AppImage \
    "$bundle"/msi/*.msi "$bundle"/nsis/*.exe "$bundle"/dmg/*.dmg)
  if [[ ${#found[@]} -eq 0 ]]; then
    echo "::error::no installers under $bundle"
    exit 1
  fi
  cp "${found[@]}" dist/app/
  ;;
*)
  echo "usage: package.sh cli <rust-target> <version> | app <rust-target>" >&2
  exit 2
  ;;
esac
ls -l dist/*/
