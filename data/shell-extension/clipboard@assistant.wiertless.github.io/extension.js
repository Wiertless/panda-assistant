// Assistant Clipboard: a short, in-memory history of copied text that only
// Assistant's service (assistantd) may read, over D-Bus on GNOME Shell's
// connection. On Wayland only the focused app can read the clipboard, so the
// shell is the one place that sees every copy.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const PATH = '/io/github/wiertless/PandaAssistant/Clipboard';
const IFACE = `<node><interface name="io.github.wiertless.PandaAssistant.Clipboard1">
  <method name="History"><arg type="as" name="clips" direction="out"/></method>
  <method name="Clear"/>
</interface></node>`;
const KEEP = 20;
const MAX_CHARS = 4000;
// Password managers mark secrets with this (KeePassXC, Bitwarden, 1Password…).
const SECRET = 'x-kde-passwordManagerHint';
const TEXT = ['text/plain', 'text/plain;charset=utf-8', 'UTF8_STRING', 'STRING', 'TEXT'];

class History {
    constructor() {
        this.clips = [];
    }

    add(text) {
        if (!text || !text.trim())
            return;
        text = text.slice(0, MAX_CHARS);
        this.clips = [text, ...this.clips.filter(c => c !== text)].slice(0, KEEP);
    }

    // Only assistantd gets an answer: other apps on the bus can't read it.
    _allowed(invocation) {
        try {
            const reply = Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
                new GLib.Variant('(s)', [invocation.get_sender()]), new GLib.VariantType('(u)'),
                Gio.DBusCallFlags.NONE, 1000, null);
            const [pid] = reply.deepUnpack();
            return GLib.path_get_basename(GLib.file_read_link(`/proc/${pid}/exe`)) === 'assistantd';
        } catch (e) {
            return false;
        }
    }

    HistoryAsync(_params, invocation) {
        if (!this._allowed(invocation)) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied', 'Only Assistant may read this');
            return;
        }
        invocation.return_value(new GLib.Variant('(as)', [this.clips]));
    }

    ClearAsync(_params, invocation) {
        if (!this._allowed(invocation)) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied', 'Only Assistant may clear this');
            return;
        }
        this.clips = [];
        invocation.return_value(null);
    }
}

export default class AssistantClipboard extends Extension {
    enable() {
        this._history = new History();
        this._selection = global.display.get_selection();
        this._changed = this._selection.connect('owner-changed', (_sel, type, source) => {
            if (type !== Meta.SelectionType.SELECTION_CLIPBOARD || !source)
                return;
            const mimes = source.get_mimetypes();
            if (mimes.includes(SECRET) || !mimes.some(m => TEXT.includes(m)))
                return;
            St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (_c, text) => this._history?.add(text));
        });
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this._history);
        this._dbus.export(Gio.DBus.session, PATH);
    }

    disable() {
        this._selection?.disconnect(this._changed);
        this._dbus?.unexport();
        this._selection = null;
        this._changed = null;
        this._dbus = null;
        this._history = null; // forgotten on lock and logout
    }
}
