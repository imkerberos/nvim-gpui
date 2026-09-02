#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: verify-no-nix-deps.sh APP_BUNDLE" >&2
  exit 2
fi

app_bundle="$1"
found_dependency=0

while IFS= read -r -d '' binary; do
  if otool -L "$binary" | grep -q '/nix/store/'; then
    echo "error: $binary still references the Nix store:" >&2
    otool -L "$binary" >&2
    found_dependency=1
  fi
done < <(find "$app_bundle/Contents" -type f -perm -111 -print0)

if [ "$found_dependency" -ne 0 ]; then
  exit 1
fi

echo "verified: no AppBundle executable references /nix/store"
