#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

ARCH="${ARCH:-$(uname -m)}"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n1)}"
BUILD_DIR="${BUILD_DIR:-$ROOT_DIR/target/appimage}"
APPDIR="$BUILD_DIR/Neutron.AppDir"
TOOLS_DIR="${TOOLS_DIR:-$ROOT_DIR/.cache/appimage-tools}"
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist}"

export APPIMAGE_EXTRACT_AND_RUN=1
export ARCH
export LDAI_OUTPUT="$DIST_DIR/Neutron-${VERSION}-${ARCH}.AppImage"

mkdir -p "$BUILD_DIR" "$TOOLS_DIR" "$DIST_DIR"
rm -rf "$APPDIR"

if [[ ! -x "$ROOT_DIR/target/release/neutron" ]] || [[ "${1:-}" == "--build" ]]; then
  echo "==> Building release binary..."
  (cd "$ROOT_DIR" && cargo build --release --locked)
fi

echo "==> Staging AppDir..."
make -C "$ROOT_DIR" DESTDIR="$APPDIR" PREFIX=/usr install
# Remove dangling absolute symlink to /usr/bin/neutron inside AppDir
rm -f "$APPDIR/usr/share/steam/compatibilitytools.d/neutron/neutron"

# AppImage root desktop entry & icon
cp "$APPDIR/usr/share/applications/com.github.romulof.neutron.desktop" "$APPDIR/com.github.romulof.neutron.desktop"
cp "$APPDIR/usr/share/icons/hicolor/scalable/apps/neutron.svg" "$APPDIR/neutron.svg"
ln -sf "neutron.svg" "$APPDIR/.DirIcon"

download_tool() {
  local url="$1"
  local dest="$2"
  if [[ ! -x "$dest" ]]; then
    echo "==> Downloading $(basename "$dest")..."
    curl -fsSL "$url" -o "$dest"
    chmod +x "$dest"
  fi
}

LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-${ARCH}.AppImage"
GTK_PLUGIN="$TOOLS_DIR/linuxdeploy-plugin-gtk.sh"

download_tool \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${ARCH}.AppImage" \
  "$LINUXDEPLOY"

download_tool \
  "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh" \
  "$GTK_PLUGIN"

chmod +x "$SCRIPT_DIR/AppRun"

echo "==> Packaging AppImage: $LDAI_OUTPUT"
rm -f "$LDAI_OUTPUT" "$LDAI_OUTPUT.sha256"
export DEPLOY_GTK_VERSION=4
export NO_STRIP="${NO_STRIP:-1}"

PATH="$TOOLS_DIR:$PATH" "$LINUXDEPLOY" \
  --appdir "$APPDIR" \
  --executable "$APPDIR/usr/bin/neutron" \
  --desktop-file "$APPDIR/com.github.romulof.neutron.desktop" \
  --icon-file "$APPDIR/neutron.svg" \
  --custom-apprun "$SCRIPT_DIR/AppRun" \
  --plugin gtk \
  --output appimage

# linuxdeploy may name output Neutron-<arch>.AppImage depending on appimagetool version;
# normalize to Neutron-<version>-<arch>.AppImage if needed.
if [[ ! -f "$LDAI_OUTPUT" ]]; then
  generated="$(find "$PWD" "$BUILD_DIR" -maxdepth 2 -name "Neutron*.AppImage" -type f -print -quit 2>/dev/null || true)"
  if [[ -n "$generated" && "$generated" != "$LDAI_OUTPUT" ]]; then
    mv "$generated" "$LDAI_OUTPUT"
  fi
fi

if [[ ! -f "$LDAI_OUTPUT" ]]; then
  echo "Error: expected AppImage not found at $LDAI_OUTPUT" >&2
  exit 1
fi

chmod +x "$LDAI_OUTPUT"
(cd "$DIST_DIR" && sha256sum "$(basename "$LDAI_OUTPUT")" > "$(basename "$LDAI_OUTPUT").sha256")

echo "==> Done: $LDAI_OUTPUT"
