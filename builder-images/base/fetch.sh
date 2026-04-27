#!/usr/bin/env bash
# fetch <name> <url> <sha256> <dest>
# Verifies sha256 strictly (mismatch = exit 2 with both values printed).
set -euo pipefail

name="$1"
url="$2"
sha_expected="$3"
dest="$4"

mkdir -p "$(dirname "$dest")"
if [[ -s "$dest" ]]; then
  sha_actual=$(sha256sum "$dest" | cut -d' ' -f1)
  if [[ "$sha_actual" == "$sha_expected" ]]; then
    echo "$name: cached + verified"
    exit 0
  fi
  rm -f "$dest"
fi

echo "$name: fetching $url"
curl -fsSL --retry 3 --retry-delay 2 -o "$dest.partial" "$url"
mv "$dest.partial" "$dest"

sha_actual=$(sha256sum "$dest" | cut -d' ' -f1)
if [[ "$sha_actual" != "$sha_expected" ]]; then
  echo "$name: SHA256 MISMATCH" >&2
  echo "  url:      $url" >&2
  echo "  expected: $sha_expected" >&2
  echo "  actual:   $sha_actual" >&2
  exit 2
fi
echo "$name: verified"
