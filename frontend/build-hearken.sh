#!/bin/bash
# Builds Hearken (release) on this Mac, and optionally installs it in /Applications.
#
#   ./build-hearken.sh            build only
#   ./build-hearken.sh --install  build, then replace /Applications/Hearken.app (asks Hearken to quit first)
#
# Signing: if an "Apple Development" certificate is in your keychain (Xcode → Settings →
# Accounts → add your Apple ID → Manage Certificates → + Apple Development), the app is
# signed with it, so macOS keeps Hearken's microphone, screen and calendar permissions
# across rebuilds. Without one it's signed ad hoc and macOS may ask again after each build.
set -euo pipefail
cd "$(dirname "$0")"

# llama.cpp's bundled HTTP library doesn't build against Homebrew's OpenSSL 4; the
# sidecar only runs local models, so it doesn't need HTTPS.
export CMAKE_DISABLE_FIND_PACKAGE_OpenSSL=ON
source "$HOME/.cargo/env" 2>/dev/null || true

IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null | grep -m1 -o '"Apple Development: [^"]*"' | tr -d '"' || true)
if [ -n "$IDENTITY" ]; then
    echo "Signing with: $IDENTITY"
    export APPLE_SIGNING_IDENTITY="$IDENTITY"
else
    echo "No Apple Development certificate found: signing ad hoc (macOS may ask for permissions again)."
fi

./build-gpu.sh

APP="../target/release/bundle/macos/Hearken.app"
[ -d "$APP" ] || { echo "Build finished but $APP is missing"; exit 1; }
codesign --verify --deep --strict "$APP" && echo "Signature OK"

if [ "${1:-}" = "--install" ]; then
    if pgrep -f "Hearken.app/Contents/MacOS/hearken" >/dev/null; then
        if find "$HOME/Movies/hearken-recordings" -maxdepth 2 -name ".checkpoints*" -mmin -2 2>/dev/null | grep -q .; then
            echo "Hearken looks like it's recording. Stop the recording, then run this again."
            exit 1
        fi
        echo "Asking Hearken to quit…"
        osascript -e 'quit app "Hearken"' || true
        for _ in $(seq 1 30); do pgrep -f "Hearken.app/Contents/MacOS/hearken" >/dev/null || break; sleep 1; done
    fi
    rm -rf /Applications/Hearken.app
    ditto "$APP" /Applications/Hearken.app
    echo "Installed /Applications/Hearken.app"
    open /Applications/Hearken.app
fi
