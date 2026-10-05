#!/bin/sh
# Print source line counts (Rust, WGSL, ISF) and release binary size. Run from anywhere.
# SLOC skips blank lines and lines that start with a comment (//, /*, *).
set -e
cd "$(dirname "$0")/.."

count() {
	files=$(git ls-files "*.$1" | wc -l)
	raw=$(git ls-files -z "*.$1" | xargs -0 cat | wc -l)
	sloc=$(git ls-files -z "*.$1" | xargs -0 cat | grep -vE '^\s*$' | grep -cvE '^\s*(//|/\*|\*)')
	printf '%-6s %6d files %8d sloc %8d lines\n' "$1" "$files" "$sloc" "$raw"
}

echo "source:"
count rs
count wgsl
count fs

cargo build --release --quiet
bin=target/release/tripslop
stripped=$(mktemp)
strip -o "$stripped" "$bin"
mb() { echo "$(wc -c < "$1") / 1048576" | bc -l | xargs printf '%.1f MB'; }
echo "binary:"
echo "  release   $(mb "$bin")"
echo "  stripped  $(mb "$stripped")"
rm -f "$stripped"
