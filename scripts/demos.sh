#!/bin/sh
# Records the animations of the README and the site with the demo mode
# (`blinker --demo`, an off-screen window playing a script; the mouse and
# keyboard are left alone):
#
#   Windows   docs/*.gif (the README, 50 fps) and site/assets/*.mp4 (60 fps)
#   macOS     site/assets/*-mac.mp4: Mac visitors of the site get these
#
#   scripts/demos.sh
#
# Needs ffmpeg on PATH.

set -eu
cd "$(dirname "$0")/.."

# the dev build: the release one may be running as the tray
cargo build
case "$(uname -s)" in
    Darwin) BIN=target/debug/blinker; MAC=1 ;;
    *) BIN=target/debug/blinker.exe; MAC=0 ;;
esac
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

for lang in ru en; do
    for kind in demo record replay; do
        dir="$TMP/$kind-$lang"
        if [ "$kind" = demo ]; then
            "$BIN" --demo "$dir" "$lang"
        else
            "$BIN" --demo "$dir" "$lang" "$kind"
        fi
        if [ "$MAC" = 1 ]; then
            out="site/assets/$kind-$lang-mac.mp4"
        else
            out="site/assets/$kind-$lang.mp4"
            # GIF frames last whole hundredths of a second: 50 fps is the most
            # browsers play; one palette, only the changed parts of a frame
            ffmpeg -v error -y -framerate 60 -i "$dir/%04d.png" \
                -vf "fps=50,split[a][b];[a]palettegen=max_colors=256:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
                -loop 0 "docs/$kind-$lang.gif"
        fi
        ffmpeg -v error -y -framerate 60 -i "$dir/%04d.png" \
            -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p -movflags +faststart -an "$out"
        echo "$kind $lang: $out"
    done
done
