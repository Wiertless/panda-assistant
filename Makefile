PREFIX  ?= /usr
DESTDIR ?=
BINDIR  := $(PREFIX)/bin
DATADIR := $(PREFIX)/share
APP_ID  := io.github.wiertless.PandaAssistant
# Newer libadwaita widgets where the system has them (Fedora).
FEATURES ?= panda-assistant/adw-1-8

.PHONY: all build test lint install uninstall dist

all: build

build:
	cargo build --release --features $(FEATURES)

test:
	cargo test
	python3 .github/scripts/eval_verdict.py --test

lint:
	cargo clippy --all-targets --features $(FEATURES) -- -D warnings

install:
	PREFIX=$(PREFIX) DESTDIR=$(DESTDIR) ./install.sh

uninstall:
	rm -f $(DESTDIR)/etc/xdg/autostart/$(APP_ID).Reminders.desktop
	rm -rf $(DESTDIR)$(PREFIX)/libexec/panda-assistant \
		$(DESTDIR)$(DATADIR)/gnome-shell/extensions/clipboard@assistant.wiertless.github.io
	rm -f $(DESTDIR)$(BINDIR)/panda-assistant \
		$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop \
		$(DESTDIR)$(DATADIR)/gnome-shell/search-providers/$(APP_ID).search-provider.ini \
		$(DESTDIR)$(DATADIR)/dbus-1/services/$(APP_ID).service \
		$(DESTDIR)$(DATADIR)/dbus-1/services/$(APP_ID).Core.service \
		$(DESTDIR)$(DATADIR)/dbus-1/services/$(APP_ID).SearchProvider.service \
		$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml \
		$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg

# Sources plus the release binaries (core-bin/), for SilverPanda's image build.
dist:
	test -n "$(VERSION)"
	test -x target/release/panda-assistant && test -x target/release/assistantd
	rm -rf dist-tmp && mkdir -p dist-tmp
	git archive --format=tar --prefix=panda-assistant-$(VERSION)/ HEAD | tar -x -C dist-tmp
	install -D -m 0755 target/release/assistantd dist-tmp/panda-assistant-$(VERSION)/core-bin/assistantd
	install -D -m 0755 target/release/panda-assistant dist-tmp/panda-assistant-$(VERSION)/core-bin/panda-assistant
	tar -C dist-tmp -czf panda-assistant-$(VERSION).tar.gz --owner=0 --group=0 panda-assistant-$(VERSION)
	rm -rf dist-tmp
