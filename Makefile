PREFIX ?= /usr
DESTDIR ?=
CARGO ?= cargo

USER_PREFIX ?= $(HOME)/.local
STEAM_COMPAT_DIR ?= $(PREFIX)/share/steam/compatibilitytools.d/neutron
STEAM_USER_COMPAT_DIR ?= $(USER_PREFIX)/share/Steam/compatibilitytools.d/neutron

BIN_NAME := neutron
APP_ID := com.github.romulof.neutron

.PHONY: all build test check install uninstall install-user uninstall-user appimage clean

all: build

build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test --locked
	@if command -v desktop-file-validate >/dev/null 2>&1; then \
		desktop-file-validate desktop/neutron.desktop; \
	fi

install:
	install -Dm755 target/release/$(BIN_NAME) "$(DESTDIR)$(PREFIX)/bin/$(BIN_NAME)"
	install -Dm644 desktop/neutron.desktop "$(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop"
	install -Dm644 icons/neutron.svg "$(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/neutron.svg"
	install -Dm644 icons/neutron.svg "$(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg"
	install -Dm644 compat/compatibilitytool.vdf "$(DESTDIR)$(STEAM_COMPAT_DIR)/compatibilitytool.vdf"
	install -Dm644 compat/toolmanifest.vdf "$(DESTDIR)$(STEAM_COMPAT_DIR)/toolmanifest.vdf"
	ln -sf "$(PREFIX)/bin/$(BIN_NAME)" "$(DESTDIR)$(STEAM_COMPAT_DIR)/$(BIN_NAME)"
	install -Dm644 LICENSE "$(DESTDIR)$(PREFIX)/share/licenses/$(BIN_NAME)/LICENSE"

uninstall:
	rm -f "$(DESTDIR)$(PREFIX)/bin/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop"
	rm -f "$(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/neutron.svg"
	rm -f "$(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg"
	rm -rf "$(DESTDIR)$(STEAM_COMPAT_DIR)"
	rm -rf "$(DESTDIR)$(PREFIX)/share/licenses/$(BIN_NAME)"

install-user: build
	install -Dm755 target/release/$(BIN_NAME) "$(USER_PREFIX)/bin/$(BIN_NAME)"
	install -Dm644 desktop/neutron.desktop "$(USER_PREFIX)/share/applications/$(APP_ID).desktop"
	install -Dm644 icons/neutron.svg "$(USER_PREFIX)/share/icons/hicolor/scalable/apps/neutron.svg"
	install -Dm644 icons/neutron.svg "$(USER_PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg"
	install -Dm644 compat/compatibilitytool.vdf "$(STEAM_USER_COMPAT_DIR)/compatibilitytool.vdf"
	install -Dm644 compat/toolmanifest.vdf "$(STEAM_USER_COMPAT_DIR)/toolmanifest.vdf"
	ln -sf "$(USER_PREFIX)/bin/$(BIN_NAME)" "$(STEAM_USER_COMPAT_DIR)/$(BIN_NAME)"
	@if command -v update-desktop-database >/dev/null 2>&1; then \
		update-desktop-database "$(USER_PREFIX)/share/applications" >/dev/null 2>&1 || true; \
	fi
	@if command -v gtk-update-icon-cache >/dev/null 2>&1; then \
		gtk-update-icon-cache -f -t "$(USER_PREFIX)/share/icons/hicolor" >/dev/null 2>&1 || true; \
	fi

uninstall-user:
	rm -f "$(USER_PREFIX)/bin/$(BIN_NAME)"
	rm -f "$(USER_PREFIX)/share/applications/$(APP_ID).desktop"
	rm -f "$(USER_PREFIX)/share/icons/hicolor/scalable/apps/neutron.svg"
	rm -f "$(USER_PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg"
	rm -rf "$(STEAM_USER_COMPAT_DIR)"

appimage: build
	./packaging/appimage/build-appimage.sh

clean:
	$(CARGO) clean
