#!/bin/sh
# Record a short clip of each movement in isolation (see src/solo.rs).
# Usage: tools/record_solos.sh OUT_DIR [SECONDS] [solo names…]
# Names: `all` (the bubbles, everything on), `glass` (the window, everything
# on), or a control (`glass.breath`, `wind_speed`; see `--solo list`).
# Needs ffmpeg.
set -e
out="$1"; secs="${2:-10}"; shift 2 || shift $#
mkdir -p "$out"
cargo build --release -q
names="$*"
[ -n "$names" ] || names="all $(cargo run --release -q -- --solo list | awk '{print $1}')"
for name in $names; do
    frames="$out/frames_$name"
    rm -rf "$frames"; mkdir -p "$frames"
    case "$name" in
        all) solo="--synthetic --no-prefs --scene bubbles" ;;
        glass) solo="--synthetic --no-prefs --scene glass" ;;
        *) solo="--solo $name" ;;
    esac
    # 1280×720 at one pixel per point, time stepped exactly 1/24 s per frame,
    # slow synthetic changes 3× faster so they fit in a short clip.
    # A covered window renders black frames (tiny PNGs): retry up to 3 times.
    for attempt in 1 2 3; do
        rm -rf "$frames"; mkdir -p "$frames"
        cargo run --release -q -- $solo --bubbles 3500 --synth-speed 3 \
            --width 1280 --height 720 --scale-factor 1 --fixed-fps 24 \
            --bench "$secs" --capture "$frames" --capture-every 0.04166 >/dev/null 2>&1
        black=$(find "$frames" -name '*.png' -size -100k | wc -l | tr -d ' ')
        [ "$black" = 0 ] && break
        echo "$name: $black black frames (attempt $attempt), retrying" >&2
    done
    ffmpeg -loglevel error -y -framerate 24 -pattern_type glob -i "$frames/frame_*.png" \
        -c:v libx264 -crf 20 -pix_fmt yuv420p -movflags +faststart "$out/$name.mp4"
    rm -rf "$frames"
done
