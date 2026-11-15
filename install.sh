#!/bin/sh
# Install Assistant. Usage: ./install.sh [PREFIX] (default /usr); DESTDIR is honoured.
# Release tarballs carry prebuilt binaries in core-bin/; in a checkout, build
# them first with: cargo build --release --features panda-assistant/adw-1-8
set -eu
PREFIX=${1:-${PREFIX:-/usr}}
DESTDIR=${DESTDIR:-}
APP_ID=io.github.wiertless.PandaAssistant
BINDIR=$PREFIX/bin
DATADIR=$PREFIX/share
LIBEXECDIR=$PREFIX/libexec/panda-assistant
cd "$(dirname "$0")"

pick() {
  for f in "core-bin/$1" "target/release/$1"; do
    if [ -x "$f" ]; then echo "$f"; return; fi
  done
  echo "missing $1: build it with cargo build --release" >&2
  exit 1
}
install -D -m 0755 "$(pick panda-assistant)" "$DESTDIR$BINDIR/panda-assistant"
install -D -m 0755 "$(pick assistantd)" "$DESTDIR$LIBEXECDIR/assistantd"
install -D -m 0644 "data/$APP_ID.desktop" "$DESTDIR$DATADIR/applications/$APP_ID.desktop"
install -D -m 0644 "data/$APP_ID.metainfo.xml" "$DESTDIR$DATADIR/metainfo/$APP_ID.metainfo.xml"
install -D -m 0644 LICENSE "$DESTDIR$DATADIR/licenses/panda-assistant/LICENSE"
# The clipboard history extension (enable it in Extensions, or SilverPanda does).
ext=clipboard@assistant.wiertless.github.io
install -D -m 0644 "data/shell-extension/$ext/metadata.json" "$DESTDIR$DATADIR/gnome-shell/extensions/$ext/metadata.json"
install -D -m 0644 "data/shell-extension/$ext/extension.js" "$DESTDIR$DATADIR/gnome-shell/extensions/$ext/extension.js"
install -D -m 0644 "data/$APP_ID.svg" "$DESTDIR$DATADIR/icons/hicolor/scalable/apps/$APP_ID.svg"
install -D -m 0644 "data/$APP_ID.search-provider.ini" \
  "$DESTDIR$DATADIR/gnome-shell/search-providers/$APP_ID.search-provider.ini"
# The window, the service, and GNOME search (served by the service).
for svc in "$APP_ID.service" "$APP_ID.Core.service" "$APP_ID.SearchProvider.service"; do
  install -D -m 0644 "data/$svc" "$DESTDIR$DATADIR/dbus-1/services/$svc"
  sed -i "s|@BINDIR@|$BINDIR|; s|@LIBEXECDIR@|$LIBEXECDIR|" "$DESTDIR$DATADIR/dbus-1/services/$svc"
done
