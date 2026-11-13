# arch-launcher — build & install helpers.
#   make                       build release binary
#   make test                  run the test suite
#   sudo make install          install to /usr/local (PREFIX=/usr for packaging)
#   make install PREFIX=$HOME/.local   per-user install, no root
#   sudo make uninstall        remove installed files (same PREFIX as install)

PREFIX  ?= /usr/local
DESTDIR ?=
APP_ID  := io.github.archlauncher.ArchLauncher
BIN     := target/release/arch-launcher

BINDIR  := $(DESTDIR)$(PREFIX)/bin
APPDIR  := $(DESTDIR)$(PREFIX)/share/applications
ICONDIR := $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
DOCDIR  := $(DESTDIR)$(PREFIX)/share/doc/arch-launcher
LICDIR  := $(DESTDIR)$(PREFIX)/share/licenses/arch-launcher

# System caches are refreshed only for direct installs; packages (DESTDIR set)
# must not ship them — pacman hooks regenerate them on the target system.
define refresh_caches
@if [ -z "$(DESTDIR)" ]; then \
	gtk4-update-icon-cache -q -t -f $(PREFIX)/share/icons/hicolor 2>/dev/null || true; \
	update-desktop-database -q $(APPDIR) 2>/dev/null || true; \
fi
endef

.PHONY: all build test install uninstall clean

all: build

build:
	cargo build --release --locked

test:
	cargo test --locked

install:
	@test -x $(BIN) || { echo "Run 'make' first (as your user, not root)."; exit 1; }
	install -Dm755 $(BIN) $(BINDIR)/arch-launcher
	install -Dm644 data/$(APP_ID).desktop $(APPDIR)/$(APP_ID).desktop
	# Absolute Exec: ~/.local/bin is often missing from the graphical session's PATH.
	sed -i 's|^Exec=arch-launcher|Exec=$(PREFIX)/bin/arch-launcher|' $(APPDIR)/$(APP_ID).desktop
	install -Dm644 data/icons/$(APP_ID).svg $(ICONDIR)/$(APP_ID).svg
	install -Dm644 README.md $(DOCDIR)/README.md
	install -Dm644 ARCHITECTURE.md $(DOCDIR)/ARCHITECTURE.md
	install -Dm644 examples/config.toml $(DOCDIR)/examples/config.toml
	install -Dm644 examples/style.css $(DOCDIR)/examples/style.css
	install -Dm644 examples/hyprland.conf $(DOCDIR)/examples/hyprland.conf
	install -Dm644 examples/sway.conf $(DOCDIR)/examples/sway.conf
	install -Dm644 LICENSE $(LICDIR)/LICENSE
	$(refresh_caches)

uninstall:
	rm -f $(BINDIR)/arch-launcher
	rm -f $(APPDIR)/$(APP_ID).desktop
	rm -f $(ICONDIR)/$(APP_ID).svg
	rm -rf $(DOCDIR) $(LICDIR)
	$(refresh_caches)
	@echo "User data was kept: ~/.config/arch-launcher and ~/.local/state/arch-launcher"

clean:
	cargo clean
