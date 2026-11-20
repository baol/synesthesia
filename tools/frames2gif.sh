#!/bin/sh
# Turn captured frames (`--capture DIR --capture-every 0.04166 --fixed-fps 24`)
# into a looping GIF, e.g. for the README.
# Usage: tools/frames2gif.sh FRAMES_DIR OUT.gif [FPS] [WIDTH] [SECONDS] [COLOURS]
# (The README's bubbles use 8 480 3.75 64; the glass 8 520 4.5.)
# Needs ffmpeg.
set -e
[ $# -ge 2 ] || { sed -n 2,6p "$0"; exit 2; }
fps="${3:-8}"; width="${4:-520}"; secs="${5:-4.5}"; colours="${6:-256}"
# One palette for the whole clip (from what changes), then dithered to it.
ffmpeg -loglevel error -y -framerate 24 -pattern_type glob -i "$1/frame_*.png" -t "$secs" \
    -vf "fps=$fps,scale=$width:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=$colours:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4" \
    -loop 0 "$2"
