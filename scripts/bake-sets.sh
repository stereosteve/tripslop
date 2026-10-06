#!/bin/sh
# Render the welcome screen's picture of each bundled set (sets/thumbs/NN-name.jpg): open the
# set, let it run, and save a 640-pixel-wide JPEG of the output. Run from the repo root after
# adding or changing a set, then rebuild (build.rs bundles the pictures):
#   scripts/bake-sets.sh            # every set
#   scripts/bake-sets.sh 03-deep-field
# Pick the scene and moment per set in the `case` below; the default is scene 1 after 5 s.
set -e
cd "$(dirname "$0")/.."
cargo build --release
mkdir -p sets/thumbs target/script-out
for file in sets/*.tripset; do
	stem=$(basename "$file" .tripset)
	[ $# -gt 0 ] && case " $* " in *" $stem "*) ;; *) continue ;; esac
	scene=1
	secs=5
	case "$stem" in
	02-patch-bay) secs=5 ;;
	04-warehouse) scene=2; secs=6 ;;
	05-showroom) scene=1; secs=4 ;;
	esac
	script=target/script-out/bake-$stem.tripslop
	cat >"$script" <<SCRIPT
open $file
quantize off
launch-scene $scene
at ${secs}s snapshot sets/thumbs/$stem.jpg 640
at +0.1s quit
SCRIPT
	./target/release/tripslop --script "$script"
done
ls -l sets/thumbs
