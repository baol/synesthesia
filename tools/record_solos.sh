#!/bin/sh
# Record review clips (see src/solo.rs) and the list the review page builds
# its cards from (OUT_DIR/clips.js, from `--solo json`).
# Usage: tools/record_solos.sh OUT_DIR [SECONDS] [names…]
# Names: a scene (`bubbles`, `glass`, … : everything on) or a control
# (`glass.breath`; see `--solo list`). No names: every scene and control.
# Needs ffmpeg.
set -e
[ $# -ge 1 ] || { sed -n 2,7p "$0"; exit 2; }
out=$(mkdir -p "$1" && cd "$1" && pwd); secs="${2:-10}"; shift 2 || shift $#
cd "$(dirname "$0")/.."
cargo build --release -q
app=target/release/synestesia
export BEVY_ASSET_ROOT="$PWD"  # (run directly, the app looks for assets/ beside itself)
# The page's card list, always current with the app.
{ printf 'window.CLIPS = '; "$app" --solo json; printf ';\n'; } > "$out/clips.js"
names="$*"
[ -n "$names" ] || names=$("$app" --solo json | sed -n 's/.*"name": "\([^"]*\)".*/\1/p')
for name in $names; do
    case "$name" in
        *.*) solo="--solo $name" ;;
        *) solo="--synthetic --no-prefs --scene $name" ;;
    esac
    frames="$out/frames_$name"
    # 1280×720 at one pixel per point, time stepped exactly 1/24 s per frame,
    # slow synthetic changes 3× faster so they fit in a short clip.
    # A covered window renders black frames (tiny PNGs): retry up to 3 times.
    for attempt in 1 2 3; do
        rm -rf "$frames"; mkdir -p "$frames"
        "$app" $solo --bubbles 3500 --synth-speed 3 \
            --width 1280 --height 720 --scale-factor 1 --fixed-fps 24 \
            --bench "$secs" --capture "$frames" --capture-every 0.04166 >/dev/null 2>&1
        black=$(find "$frames" -name '*.png' -size -20k | wc -l | tr -d ' ')
        [ "$black" = 0 ] && break
        echo "$name: $black black frames (attempt $attempt), retrying" >&2
    done
    ffmpeg -loglevel error -y -framerate 24 -pattern_type glob -i "$frames/frame_*.png" \
        -c:v libx264 -crf 20 -pix_fmt yuv420p -movflags +faststart "$out/$name.mp4"
    rm -rf "$frames"
    echo "$name"
done
