# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
via [Conventional Commits](https://www.conventionalcommits.org/).

## [1.0.0] - 2026-11-22

### Features

- **manager:** Standalone GTK4 / Libadwaita settings manager for `global.env` and per-game Steam Proton overrides (`~/.config/neutron/*.env`).
- **managed:** Built-in toggles and editors for Proton version, launcher timeout, GameMode, MangoHud, GameScope, SMT disabling, VRAM headroom reservation, shader cache size, and DLSS presets.
- **launcher:** Pre-launch countdown window (`waitforexitandrun` and `launch %command%`) with F2 quick-configure, immediate launch, and cancel actions.
- **packaging:** Arch Linux / AUR `PKGBUILD` definitions, `Makefile` installation targets, and standalone `x86_64` AppImage packaging.
