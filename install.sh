#!/bin/sh
# Install Panda. Usage: ./install.sh [PREFIX] (default /usr); DESTDIR is honoured.
set -eu
PREFIX=${1:-${PREFIX:-/usr}}
DESTDIR=${DESTDIR:-}
APP_ID=io.github.wiertless.PandaAssistant
BINDIR=$PREFIX/bin
DATADIR=$PREFIX/share
LIBDIR=$DATADIR/panda-assistant
LIBEXECDIR=$PREFIX/libexec/panda-assistant
cd "$(dirname "$0")"

# The service that runs the model and tools (Rust). Release tarballs carry a
# prebuilt one; in a checkout, build it with: cargo build --release --manifest-path core/Cargo.toml
for core in core-bin/assistantd core/target/release/assistantd; do
  if [ -x "$core" ]; then
    install -D -m 0755 "$core" "$DESTDIR$LIBEXECDIR/assistantd"
    install -D -m 0644 "data/$APP_ID.Core.service" "$DESTDIR$DATADIR/dbus-1/services/$APP_ID.Core.service"
    sed -i "s|@LIBEXECDIR@|$LIBEXECDIR|" "$DESTDIR$DATADIR/dbus-1/services/$APP_ID.Core.service"
    break
  fi
done
[ -x "$DESTDIR$LIBEXECDIR/assistantd" ] || echo "warning: assistantd isn't built; the app needs it" >&2

install -d "$DESTDIR$LIBDIR/panda_assistant"
install -m 0644 src/panda_assistant/*.py "$DESTDIR$LIBDIR/panda_assistant/"
install -D -m 0755 bin/panda-assistant "$DESTDIR$BINDIR/panda-assistant"
sed -i "s|@LIBDIR@|$LIBDIR|" "$DESTDIR$BINDIR/panda-assistant"
install -D -m 0644 "data/$APP_ID.desktop" "$DESTDIR$DATADIR/applications/$APP_ID.desktop"
install -D -m 0644 "data/$APP_ID.metainfo.xml" "$DESTDIR$DATADIR/metainfo/$APP_ID.metainfo.xml"
install -D -m 0644 LICENSE "$DESTDIR$DATADIR/licenses/panda-assistant/LICENSE"
install -D -m 0644 "data/$APP_ID.svg" "$DESTDIR$DATADIR/icons/hicolor/scalable/apps/$APP_ID.svg"
install -D -m 0644 "data/$APP_ID.search-provider.ini" \
  "$DESTDIR$DATADIR/gnome-shell/search-providers/$APP_ID.search-provider.ini"
# The window, and GNOME search (served by assistantd).
for svc in "$APP_ID.service" "$APP_ID.SearchProvider.service"; do
  install -D -m 0644 "data/$svc" "$DESTDIR$DATADIR/dbus-1/services/$svc"
  sed -i "s|@BINDIR@|$BINDIR|; s|@LIBEXECDIR@|$LIBEXECDIR|" "$DESTDIR$DATADIR/dbus-1/services/$svc"
done
