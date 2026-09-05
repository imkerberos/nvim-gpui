#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: verify-no-nix-deps.sh APP_BUNDLE" >&2
  exit 2
fi

app_bundle="$1"
if [ ! -d "$app_bundle/Contents" ]; then
  echo "error: AppBundle Contents directory does not exist: $app_bundle/Contents" >&2
  exit 1
fi

found_reference=0
macho_files=0

check_nix_marker() {
  local path="$1"
  if LC_ALL=C grep -aFq '/nix/store/' "$path" ||
    LC_ALL=C grep -aFq '\nix\store\' "$path"; then
    echo "error: $path contains a Nix store reference:" >&2
    found_reference=1
  fi
}

while IFS= read -r -d '' binary; do
  if [ ! -e "$binary" ]; then
    echo "error: AppBundle contains a broken symlink: $binary" >&2
    found_reference=1
    continue
  fi

  check_nix_marker "$binary"

  file_type="$(file -bL "$binary")"
  if [[ "$file_type" != *"Mach-O"* ]]; then
    continue
  fi

  macho_files=$((macho_files + 1))
  load_commands="$(otool -l "$binary")"
  dependencies="$(otool -L "$binary")"
  if grep -Eq '/nix/store/' <<<"$load_commands$dependencies"; then
    echo "error: $binary still references the Nix store:" >&2
    printf '%s\n' "$load_commands" "$dependencies" >&2
    found_reference=1
  fi
done < <(find "$app_bundle/Contents" \( -type f -o -type l \) -print0)

if [ "$macho_files" -eq 0 ]; then
  echo "error: no Mach-O files found in AppBundle Contents" >&2
  exit 1
fi

if [ "$found_reference" -ne 0 ]; then
  exit 1
fi

echo "verified: no Nix store references in $macho_files AppBundle Mach-O files or bundled files"
