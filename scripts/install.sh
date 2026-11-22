#!/usr/bin/env bash
set -euo pipefail

REPO="romulof/neutron"
RELEASE_BASE="https://github.com/${REPO}/releases/latest/download"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/latest"

MODE=""

usage() {
  cat <<'EOF'
Neutron installer

Usage:
  install.sh [--pacman | --user | --appimage]

Modes:
  --pacman     Install prebuilt Arch package via sudo pacman -U (default on Arch/CachyOS)
  --user       Install binary, desktop entry, icons, and Steam compat tool into ~/.local
  --appimage   Install standalone x86_64 AppImage into ~/.local/bin/neutron with desktop/Steam integration
  -h, --help   Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pacman)
      MODE="pacman"
      shift
      ;;
    --user)
      MODE="user"
      shift
      ;;
    --appimage)
      MODE="appimage"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

ARCH="$(uname -m)"
if [[ "$ARCH" != "x86_64" ]]; then
  echo "Error: prebuilt releases currently target x86_64 (detected: $ARCH)." >&2
  echo "Please build from source using 'make build && sudo make install'." >&2
  exit 1
fi

if [[ -z "$MODE" ]]; then
  if command -v pacman >/dev/null 2>&1; then
    MODE="pacman"
  else
    MODE="user"
  fi
fi

for cmd in curl sha256sum; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Error: required command '$cmd' not found." >&2
    exit 1
  fi
done

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

download_and_verify() {
  local filename="$1"
  echo "==> Downloading ${filename}..."
  curl -fsSL "${RELEASE_BASE}/${filename}" -o "${TMPDIR}/${filename}"
  curl -fsSL "${RELEASE_BASE}/${filename}.sha256" -o "${TMPDIR}/${filename}.sha256"
  echo "==> Verifying SHA256 checksum..."
  (cd "$TMPDIR" && sha256sum -c "${filename}.sha256")
}

install_pacman() {
  local pkg="neutron-x86_64.pkg.tar.zst"
  download_and_verify "$pkg"
  echo "==> Installing with pacman..."
  sudo pacman -U --needed "${TMPDIR}/${pkg}"
  echo "==> Neutron installed! Restart Steam to select Neutron under Settings -> Compatibility."
}

install_user() {
  local tarball="neutron-x86_64-linux-gnu.tar.gz"
  local user_prefix="${HOME}/.local"
  local steam_compat="${user_prefix}/share/Steam/compatibilitytools.d/neutron"

  download_and_verify "$tarball"
  mkdir -p "${TMPDIR}/extract"
  tar -xzf "${TMPDIR}/${tarball}" -C "${TMPDIR}/extract"

  echo "==> Installing to ${user_prefix}..."
  install -Dm755 "${TMPDIR}/extract/usr/bin/neutron" "${user_prefix}/bin/neutron"
  install -Dm644 "${TMPDIR}/extract/usr/share/applications/com.github.romulof.neutron.desktop" \
    "${user_prefix}/share/applications/com.github.romulof.neutron.desktop"
  install -Dm644 "${TMPDIR}/extract/usr/share/icons/hicolor/scalable/apps/neutron.svg" \
    "${user_prefix}/share/icons/hicolor/scalable/apps/neutron.svg"
  install -Dm644 "${TMPDIR}/extract/usr/share/icons/hicolor/scalable/apps/com.github.romulof.neutron.svg" \
    "${user_prefix}/share/icons/hicolor/scalable/apps/com.github.romulof.neutron.svg"
  install -Dm644 "${TMPDIR}/extract/usr/share/steam/compatibilitytools.d/neutron/compatibilitytool.vdf" \
    "${steam_compat}/compatibilitytool.vdf"
  install -Dm644 "${TMPDIR}/extract/usr/share/steam/compatibilitytools.d/neutron/toolmanifest.vdf" \
    "${steam_compat}/toolmanifest.vdf"
  ln -sf "${user_prefix}/bin/neutron" "${steam_compat}/neutron"
  install -Dm644 "${TMPDIR}/extract/usr/share/licenses/neutron/LICENSE" \
    "${user_prefix}/share/licenses/neutron/LICENSE"

  refresh_desktop_caches "$user_prefix"
  echo "==> Neutron installed to ${user_prefix}/bin/neutron."
  echo "    Ensure ${user_prefix}/bin is in your PATH and restart Steam to enable the compatibility tool."
}

install_appimage() {
  local appimage="Neutron-x86_64.AppImage"
  local user_prefix="${HOME}/.local"
  local steam_compat="${user_prefix}/share/Steam/compatibilitytools.d/neutron"

  download_and_verify "$appimage"

  echo "==> Installing AppImage to ${user_prefix}/bin/neutron..."
  install -Dm755 "${TMPDIR}/${appimage}" "${user_prefix}/bin/neutron"

  curl -fsSL "${RAW_BASE}/desktop/neutron.desktop" -o "${TMPDIR}/neutron.desktop"
  curl -fsSL "${RAW_BASE}/icons/neutron.svg" -o "${TMPDIR}/neutron.svg"
  curl -fsSL "${RAW_BASE}/compat/compatibilitytool.vdf" -o "${TMPDIR}/compatibilitytool.vdf"
  curl -fsSL "${RAW_BASE}/compat/toolmanifest.vdf" -o "${TMPDIR}/toolmanifest.vdf"

  install -Dm644 "${TMPDIR}/neutron.desktop" "${user_prefix}/share/applications/com.github.romulof.neutron.desktop"
  install -Dm644 "${TMPDIR}/neutron.svg" "${user_prefix}/share/icons/hicolor/scalable/apps/neutron.svg"
  install -Dm644 "${TMPDIR}/neutron.svg" "${user_prefix}/share/icons/hicolor/scalable/apps/com.github.romulof.neutron.svg"
  install -Dm644 "${TMPDIR}/compatibilitytool.vdf" "${steam_compat}/compatibilitytool.vdf"
  install -Dm644 "${TMPDIR}/toolmanifest.vdf" "${steam_compat}/toolmanifest.vdf"
  ln -sf "${user_prefix}/bin/neutron" "${steam_compat}/neutron"

  refresh_desktop_caches "$user_prefix"
  echo "==> Neutron AppImage installed to ${user_prefix}/bin/neutron."
}

refresh_desktop_caches() {
  local prefix="$1"
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${prefix}/share/applications" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${prefix}/share/icons/hicolor" >/dev/null 2>&1 || true
  fi
}

case "$MODE" in
  pacman) install_pacman ;;
  user) install_user ;;
  appimage) install_appimage ;;
esac
