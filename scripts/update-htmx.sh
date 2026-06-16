#!/bin/sh
# Update the vendored htmx runtime to a pinned version.
#
#   scripts/update-htmx.sh 2.0.5
#
# Downloads the requested htmx release, verifies it downloaded intact, drops it
# into crates/relatum-web/static/ as htmx-<version>.min.js, and prints the line
# you must change in meta.rs. The served URL (/static/htmx.min.js) is decoupled
# from the disk filename, so layout.html needs no edit.
#
# Run from the repository root.
set -eu

if [ $# -ne 1 ]; then
	echo "usage: $0 <version>   (e.g. $0 2.0.5)" >&2
	exit 2
fi

ver="$1"
static_dir="crates/relatum-web/static"
url="https://cdn.jsdelivr.net/npm/htmx.org@${ver}/dist/htmx.min.js"
out="${static_dir}/htmx-${ver}.min.js"

if [ ! -d "$static_dir" ]; then
	echo "error: $static_dir not found — run from the repository root" >&2
	exit 1
fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

echo "downloading htmx ${ver}"
curl -fsSL "$url" -o "$tmp"

# Sanity check: a real htmx build is tens of KB and starts with its IIFE.
size="$(wc -c < "$tmp" | tr -d ' ')"
if [ "$size" -lt 10000 ]; then
	echo "error: downloaded file is only ${size} bytes — wrong version or bad URL?" >&2
	exit 1
fi
if ! head -c 64 "$tmp" | grep -q 'htmx'; then
	echo "error: downloaded file does not look like htmx" >&2
	exit 1
fi

# Remove older vendored copies so only the pinned version remains.
rm -f "${static_dir}"/htmx-*.min.js
mv "$tmp" "$out"
trap - EXIT

echo "wrote ${out} (${size} bytes)"
echo "sha256: $(sha256sum "$out" | cut -d' ' -f1)"
echo
echo "now update the include_str! path in crates/relatum-web/src/handlers/meta.rs:"
echo "    include_str!(\"../../static/htmx-${ver}.min.js\")"
