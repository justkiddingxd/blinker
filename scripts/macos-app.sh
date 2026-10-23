#!/bin/sh
# Builds target/release/Blinker.app, and next to it Blinker.dmg (opens to the
# app and an Applications shortcut) and Blinker-macos.zip (for the updater).
#
#   scripts/macos-app.sh              this Mac's architecture
#   UNIVERSAL=1 scripts/macos-app.sh  Apple silicon + Intel in one binary
#
# Signing: with the certificate from scripts/macos-signing.sh, macOS keeps the
# Screen Recording permission across rebuilds and updates. Without it the app
# is signed ad hoc and every build has to be allowed again.

set -eu
cd "$(dirname "$0")/.."

export MACOSX_DEPLOYMENT_TARGET=14.0
if [ "${UNIVERSAL:-0}" = 1 ]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p target/release
    lipo -create -output target/release/blinker-universal \
        target/aarch64-apple-darwin/release/blinker target/x86_64-apple-darwin/release/blinker
    BIN=target/release/blinker-universal
else
    cargo build --release
    BIN=target/release/blinker
fi

APP=target/release/Blinker.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/blinker"
cp assets/macos/blinker.icns "$APP/Contents/Resources/blinker.icns"
sed "s/@VERSION@/$VERSION/g" assets/macos/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
# macOS 26: the Liquid Glass icon (an Icon Composer file) next to the old
# one, when this Mac's Xcode can build it (Xcode 26's actool)
if xcrun --find actool >/dev/null 2>&1; then
    ICON_OUT=$(mktemp -d)
    if xcrun actool assets/macos/Blinker.icon --compile "$ICON_OUT" --app-icon Blinker --include-all-app-icons \
        --enable-on-demand-resources NO --development-region en --target-device mac --platform macosx \
        --minimum-deployment-target 14.0 --output-partial-info-plist "$ICON_OUT/partial.plist" >"$ICON_OUT/log" 2>&1 &&
        [ -f "$ICON_OUT/Assets.car" ]; then
        cp "$ICON_OUT/Assets.car" "$APP/Contents/Resources/Assets.car"
        /usr/libexec/PlistBuddy -c "Add :CFBundleIconName string Blinker" "$APP/Contents/Info.plist"
        ICON="Liquid Glass"
    else
        echo "warning: the Liquid Glass icon wasn't built:" >&2
        sed 's/^/    /' "$ICON_OUT/log" >&2
    fi
    rm -rf "$ICON_OUT"
else
    echo "note: no actool (Xcode 26): the app keeps the plain icon" >&2
fi
KEYCHAIN="$HOME/Library/Keychains/blinker-signing.keychain-db"
KEYCHAIN_PASSWORD_FILE="$HOME/Library/Application Support/Blinker Signing/keychain-password"
if [ -f "$KEYCHAIN" ] && [ -f "$KEYCHAIN_PASSWORD_FILE" ]; then
    security unlock-keychain -p "$(cat "$KEYCHAIN_PASSWORD_FILE")" "$KEYCHAIN"
    IDENTITY=$(security find-identity -p codesigning "$KEYCHAIN" | awk '/"Blinker Self-Signed"/ { print $2; exit }')
    # codesign only finds identities in the search list: add the keychain for
    # this signature and put the list back afterwards
    SEARCH=$(security list-keychains -d user | tr -d '"')
    restore() { security list-keychains -d user -s $SEARCH; }
    trap restore EXIT
    security list-keychains -d user -s $SEARCH "$KEYCHAIN"
    codesign --force --sign "$IDENTITY" --identifier app.blinker "$APP"
    restore
    trap - EXIT
    SIGNED="Blinker Self-Signed"
else
    echo "warning: no signing certificate (scripts/macos-signing.sh): signed ad hoc," >&2
    echo "         macOS will ask for Screen Recording again after every build" >&2
    codesign --force --sign - --identifier app.blinker "$APP"
    SIGNED="ad hoc"
fi

(cd target/release && rm -f Blinker-macos.zip && ditto -c -k --keepParent Blinker.app Blinker-macos.zip)

# the disk image people download: drag Blinker onto Applications. dmgbuild
# writes the window layout (background, icon places) without Finder, so it
# works the same on CI; it lives in its own venv under target/
DMGBUILD=target/dmgbuild/bin/dmgbuild
if [ ! -x "$DMGBUILD" ]; then
    # a half-made one (a failed pip) is made again
    rm -rf target/dmgbuild
    python3 -m venv target/dmgbuild
    target/dmgbuild/bin/pip install --quiet --disable-pip-version-check dmgbuild==1.6.7
fi
rm -f target/release/Blinker.dmg
"$DMGBUILD" -s scripts/dmg-settings.py -D app="$APP" Blinker target/release/Blinker.dmg >/dev/null
echo "built $APP ($VERSION, signed: $SIGNED, icon: ${ICON:-plain})"
codesign -d -r- "$APP" 2>&1 | sed -n 's/^designated => /requirement: /p'
