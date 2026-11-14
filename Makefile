PREFIX  ?= /usr
DESTDIR ?=
BINDIR  := $(PREFIX)/bin
DATADIR := $(PREFIX)/share
LIBDIR  := $(DATADIR)/panda-assistant
APP_ID  := io.github.wiertless.PandaAssistant
PYTHON  ?= python3

.PHONY: all test install uninstall dist

all:

test:
	PYTHONPATH=src $(PYTHON) -m unittest discover -s tests -v

install:
	PREFIX=$(PREFIX) DESTDIR=$(DESTDIR) ./install.sh

uninstall:
	rm -rf $(DESTDIR)$(LIBDIR) $(DESTDIR)$(PREFIX)/libexec/panda-assistant
	rm -f $(DESTDIR)$(BINDIR)/panda-assistant \
		$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop \
		$(DESTDIR)$(DATADIR)/gnome-shell/search-providers/$(APP_ID).search-provider.ini \
		$(DESTDIR)$(DATADIR)/dbus-1/services/$(APP_ID).SearchProvider.service \
		$(DESTDIR)$(DATADIR)/dbus-1/services/$(APP_ID).service \
		$(DESTDIR)$(DATADIR)/dbus-1/services/$(APP_ID).Core.service \
		$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml \
		$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg

dist:
	test -n "$(VERSION)"
	test -x core/target/release/assistantd
	rm -rf dist-tmp && mkdir -p dist-tmp
	git archive --format=tar --prefix=panda-assistant-$(VERSION)/ HEAD | tar -x -C dist-tmp
	install -D -m 0755 core/target/release/assistantd dist-tmp/panda-assistant-$(VERSION)/core-bin/assistantd
	tar -C dist-tmp -czf panda-assistant-$(VERSION).tar.gz --owner=0 --group=0 panda-assistant-$(VERSION)
	rm -rf dist-tmp
