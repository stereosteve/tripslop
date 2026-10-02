#!/bin/sh
# Re-download the sample media used by the demo. Run from anywhere.
set -e
cd "$(dirname "$0")"
get() { [ -f "$1" ] || curl -fSL -A "trippy-sample-fetch/0.1" -o "$1" "$2"; }
get jellyfish.mp4       https://test-videos.co.uk/vids/jellyfish/mp4/h264/360/Jellyfish_360_10s_1MB.mp4
get big-buck-bunny.mp4  https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/360/Big_Buck_Bunny_360_10s_1MB.mp4
get pillars-of-creation.jpg "https://upload.wikimedia.org/wikipedia/commons/thumb/6/68/Pillars_of_creation_2014_HST_WFC3-UVIS_full-res_denoised.jpg/960px-Pillars_of_creation_2014_HST_WFC3-UVIS_full-res_denoised.jpg"
get crab-nebula.jpg     "https://upload.wikimedia.org/wikipedia/commons/thumb/0/00/Crab_Nebula.jpg/960px-Crab_Nebula.jpg"
echo "samples ready"
