//! Calendar events and contacts from the accounts set up in GNOME.
//!
//! Events come from GNOME Shell's calendar service, the one behind the
//! top-bar calendar: it already merges every calendar and expands repeating
//! events. Contacts come from Evolution Data Server's address books over
//! D-Bus, as vCards.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;

use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const MAX_EVENTS: usize = 30;
const CAL_BUS: &str = "org.gnome.Shell.CalendarServer";
const CAL_PATH: &str = "/org/gnome/Shell/CalendarServer";
const CAL_IFACE: &str = "org.gnome.Shell.CalendarServer";
const WEEKDAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// A local calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day(pub i32, pub i32, pub i32); // year, month, day

impl Day {
    fn date(self) -> glib::DateTime {
        glib::DateTime::from_local(self.0, self.1, self.2, 0, 0, 0.0).expect("valid date")
    }
    pub fn today() -> Self {
        let now = glib::DateTime::now_local().expect("clock");
        Day(now.year(), now.month(), now.day_of_month())
    }
    fn plus(self, days: i32) -> Self {
        let d = self.date().add_days(days).expect("date in range");
        Day(d.year(), d.month(), d.day_of_month())
    }
    fn weekday(self) -> i32 {
        self.date().day_of_week() - 1 // Monday = 0
    }
    pub fn start(self) -> i64 {
        self.date().to_unix()
    }
}

/// Everyday words or an ISO date -> (first day, number of days, label).
pub fn time_range(when: &str, today: Day) -> Result<(Day, i32, String), ToolError> {
    let w = when.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let w = if w.is_empty() { "today".to_string() } else { w };
    let weekday = w.trim_start_matches("on ").trim_start_matches("next ");
    Ok(match w.as_str() {
        "today" | "now" => (today, 1, "today".into()),
        "tomorrow" => (today.plus(1), 1, w.clone()),
        "yesterday" => (today.plus(-1), 1, w.clone()),
        "this week" | "week" => (today, 7 - today.weekday(), w.clone()),
        "next week" => (today.plus(7 - today.weekday()), 7, w.clone()),
        "next 7 days" | "coming week" | "upcoming" => (today, 7, w.clone()),
        _ if WEEKDAYS.contains(&weekday) => {
            let target = WEEKDAYS.iter().position(|d| *d == weekday).unwrap() as i32;
            let ahead = (target - today.weekday()).rem_euclid(7);
            let ahead = if ahead == 0 && w.starts_with("next") { 7 } else { ahead };
            (today.plus(ahead), 1, w.clone())
        }
        _ => {
            let parts: Vec<i32> = w.split('-').filter_map(|p| p.parse().ok()).collect();
            let day = match parts.as_slice() {
                [y, m, d] if glib::DateTime::from_local(*y, *m, *d, 0, 0, 0.0).is_ok() => Day(*y, *m, *d),
                _ => return Err("when must be today, tomorrow, this week, next week, a weekday or a date like 2026-11-20".into()),
            };
            let label = day.date().format("%A %Y-%m-%d").map(|s| s.to_string()).unwrap_or_default();
            (day, 1, label)
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub summary: String,
    pub start: i64,
    pub end: i64,
}

fn local(t: i64) -> glib::DateTime {
    glib::DateTime::from_unix_local(t).expect("time in range")
}

fn all_day(e: &Event) -> bool {
    let (s, en) = (local(e.start), local(e.end));
    s.hour() == 0 && s.minute() == 0 && en.hour() == 0 && en.minute() == 0 && e.end > e.start
}

pub fn format_events(mut events: Vec<Event>, label: &str) -> String {
    if events.is_empty() {
        return if matches!(label, "today" | "tomorrow" | "yesterday") { format!("No events {label}.") } else { format!("No events ({label}).") };
    }
    events.sort_by_key(|e| (local(e.start).format("%F").map(|s| s.to_string()).unwrap_or_default(), !all_day(e), e.start));
    events.iter().take(MAX_EVENTS).map(|e| {
        let day = local(e.start).format("%a %d %b").map(|s| s.to_string()).unwrap_or_default();
        let when = if all_day(e) { "all day".to_string() } else {
            format!("{}–{}", local(e.start).format("%H:%M").unwrap(), local(e.end).format("%H:%M").unwrap())
        };
        format!("{day}, {when}: {}", if e.summary.is_empty() { "(no title)" } else { &e.summary })
    }).collect::<Vec<_>>().join("\n")
}

/// Events overlapping [since, until) from GNOME Shell's calendar service.
/// Its time range is shared with the top-bar calendar, so it's put back after.
#[allow(deprecated)] // signal_subscribe: the callback form suits a one-off wait
fn shell_events(since: i64, until: i64) -> Result<Vec<Event>, ToolError> {
    let context = glib::MainContext::new();
    context.with_thread_default(|| {
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)
            .map_err(|e| ToolError(e.message().to_string()))?;
        let prop = |name: &str| bus.call_sync(Some(CAL_BUS), CAL_PATH, "org.freedesktop.DBus.Properties", "Get",
            Some(&(CAL_IFACE, name).to_variant()), Some(glib::VariantTy::new("(v)").unwrap()), gio::DBusCallFlags::NONE,
            3000, None::<&gio::Cancellable>).ok().and_then(|v| v.child_value(0).as_variant());
        if prop("HasCalendars").and_then(|v| v.get::<bool>()) == Some(false) {
            return Ok(vec![]);
        }
        let old = (prop("Since").and_then(|v| v.get::<i64>()), prop("Until").and_then(|v| v.get::<i64>()));
        let events: Rc<RefCell<BTreeMap<String, Event>>> = Rc::default();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let quiet: Rc<RefCell<Option<glib::Source>>> = Rc::default();
        let (ev, l, q, ctx) = (events.clone(), main_loop.clone(), quiet.clone(), context.clone());
        let sub = bus.signal_subscribe(Some(CAL_BUS), Some(CAL_IFACE), Some("EventsAddedOrUpdated"), Some(CAL_PATH), None,
            gio::DBusSignalFlags::NONE, move |_, _, _, _, _, params| {
                type Raw = Vec<(String, String, i64, i64, BTreeMap<String, glib::Variant>)>;
                if let Some(list) = params.child_value(0).get::<Raw>() {
                    for (id, summary, start, end, _) in list {
                        ev.borrow_mut().insert(id, Event { summary, start, end });
                    }
                }
                // Calendars answer one after another: finish once they go quiet.
                if let Some(s) = q.borrow_mut().take() {
                    s.destroy();
                }
                let l2 = l.clone();
                let s = glib::timeout_source_new(Duration::from_millis(800), None, glib::Priority::DEFAULT, move || {
                    l2.quit();
                    glib::ControlFlow::Break
                });
                s.attach(Some(&ctx));
                *q.borrow_mut() = Some(s);
            });
        let set = |a: i64, b: i64, force: bool| bus.call_sync(Some(CAL_BUS), CAL_PATH, CAL_IFACE, "SetTimeRange",
            Some(&(a, b, force).to_variant()), None, gio::DBusCallFlags::NONE, 5000, None::<&gio::Cancellable>);
        if let Err(e) = set(since, until, true) {
            bus.signal_unsubscribe(sub);
            return Err(ToolError(format!("the calendar isn't available: {}", e.message())));
        }
        let l = main_loop.clone();
        let cap = glib::timeout_source_new(Duration::from_secs(6), None, glib::Priority::DEFAULT, move || {
            l.quit();
            glib::ControlFlow::Break
        });
        cap.attach(Some(&context));
        main_loop.run();
        cap.destroy();
        if let Some(s) = quiet.borrow_mut().take() {
            s.destroy();
        }
        bus.signal_unsubscribe(sub);
        if let (Some(a), Some(b)) = old
            && b > a {
                let _ = set(a, b, false);
            }
        let list = events.borrow().values().filter(|e| e.end > since && e.start < until).cloned().collect();
        Ok(list)
    }).map_err(|e| ToolError(e.to_string()))?
}

pub fn get_events(_: &Context, args: &Args) -> ToolResult {
    let (first, days, label) = time_range(&arg_str(args, "when"), Day::today())?;
    let events = shell_events(first.start(), first.plus(days).start())?;
    Ok(format_events(events, &label))
}

// -- contacts ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
pub struct Contact {
    pub name: String,
    pub emails: Vec<String>,
    pub phones: Vec<(String, String)>,
    pub org: String,
}

/// The fields we show, from a vCard (folded lines and TYPE parameters handled).
pub fn parse_vcard(card: &str) -> Contact {
    let mut unfolded: Vec<String> = vec![];
    for line in card.split('\n').map(|l| l.trim_end_matches('\r')) {
        if (line.starts_with(' ') || line.starts_with('\t')) && !unfolded.is_empty() {
            unfolded.last_mut().unwrap().push_str(&line[1..]);
        } else {
            unfolded.push(line.to_string());
        }
    }
    let mut c = Contact::default();
    for line in unfolded {
        let Some((head, value)) = line.split_once(':') else { continue };
        let value = value.replace("\\,", ",").replace("\\;", ";").replace("\\n", " ");
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or("").to_uppercase();
        let params: Vec<String> = parts.map(|p| p.to_lowercase()).collect();
        match name.as_str() {
            "FN" => c.name = value,
            "EMAIL" if !value.is_empty() && !c.emails.contains(&value) => c.emails.push(value),
            "TEL" if !value.is_empty() && !c.phones.iter().any(|(n, _)| *n == value) => {
                let kind = if params.iter().any(|p| p.contains("cell")) { "mobile" } else if params.iter().any(|p| p.contains("work")) { "work" }
                           else if params.iter().any(|p| p.contains("home")) { "home" } else { "" };
                c.phones.push((value, kind.into()));
            }
            "ORG" => c.org = value.split(';').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
            _ => {}
        }
    }
    c
}

pub fn format_contact(c: &Contact) -> String {
    let mut parts = vec![if c.name.is_empty() { "(no name)".to_string() } else { c.name.clone() }];
    if !c.emails.is_empty() {
        parts.push(format!("email {}", c.emails.join(", ")));
    }
    if !c.phones.is_empty() {
        parts.push(format!("phone {}", c.phones.iter().map(|(n, k)| if k.is_empty() { n.clone() } else { format!("{n} ({k})") })
            .collect::<Vec<_>>().join(", ")));
    }
    if !c.org.is_empty() {
        parts.push(c.org.clone());
    }
    parts.join(" · ")
}

/// UIDs of the enabled address books, from EDS's source registry.
fn address_books(bus: &gio::DBusConnection) -> Result<Vec<String>, ToolError> {
    type Objects = BTreeMap<glib::variant::ObjectPath, BTreeMap<String, BTreeMap<String, glib::Variant>>>;
    let reply = bus.call_sync(Some("org.gnome.evolution.dataserver.Sources5"), "/org/gnome/evolution/dataserver/SourceManager",
        "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None, Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()),
        gio::DBusCallFlags::NONE, 10000, None::<&gio::Cancellable>)
        .map_err(|_| ToolError("contacts need Evolution Data Server, which isn't running".into()))?;
    let (objects,): (Objects,) = reply.get().unwrap_or_default();
    Ok(objects.values().filter_map(|ifaces| {
        let src = ifaces.get("org.gnome.evolution.dataserver.Source")?;
        let uid = src.get("UID")?.get::<String>()?;
        let data = src.get("Data")?.get::<String>()?;
        let enabled = !data.lines().any(|l| l.trim().eq_ignore_ascii_case("Enabled=false"));
        (data.contains("[Address Book]") && enabled).then_some(uid)
    }).collect())
}

fn query_book(bus: &gio::DBusConnection, uid: &str, sexp: &str) -> Result<Vec<String>, glib::Error> {
    let (path, name): (String, String) = bus.call_sync(Some("org.gnome.evolution.dataserver.AddressBook10"),
        "/org/gnome/evolution/dataserver/AddressBookFactory", "org.gnome.evolution.dataserver.AddressBookFactory",
        "OpenAddressBook", Some(&(uid,).to_variant()), Some(glib::VariantTy::new("(ss)").unwrap()),
        gio::DBusCallFlags::NONE, 15000, None::<&gio::Cancellable>)?.get().unwrap_or_default();
    let iface = "org.gnome.evolution.dataserver.AddressBook";
    bus.call_sync(Some(&name), &path, iface, "Open", None, None, gio::DBusCallFlags::NONE, 15000, None::<&gio::Cancellable>)?;
    let cards = bus.call_sync(Some(&name), &path, iface, "GetContactList", Some(&(sexp,).to_variant()),
        Some(glib::VariantTy::new("(as)").unwrap()), gio::DBusCallFlags::NONE, 15000, None::<&gio::Cancellable>)
        .map(|v| v.child_value(0).get::<Vec<String>>().unwrap_or_default());
    let _ = bus.call_sync(Some(&name), &path, iface, "Close", None, None, gio::DBusCallFlags::NONE, 5000, None::<&gio::Cancellable>);
    cards
}

/// An EDS query that matches any field containing the text.
pub fn any_field_query(text: &str) -> String {
    format!("(contains \"x-evolution-any-field\" \"{}\")", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn find_contact(_: &Context, args: &Args) -> ToolResult {
    let name = arg_str(args, "name").trim().to_string();
    if name.is_empty() {
        return Err("say whose details to find".into());
    }
    // A private connection: each opened book belongs to the connection that opened it.
    let addr = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, None::<&gio::Cancellable>)
        .map_err(|e| ToolError(e.message().to_string()))?;
    let bus = gio::DBusConnection::for_address_sync(&addr,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None::<&gio::DBusAuthObserver>, None::<&gio::Cancellable>).map_err(|e| ToolError(e.message().to_string()))?;
    let mut found: Vec<String> = vec![];
    for uid in address_books(&bus)? {
        // An account that's offline or broken shouldn't hide the rest.
        for card in query_book(&bus, &uid, &any_field_query(&name)).unwrap_or_default() {
            let line = format_contact(&parse_vcard(&card));
            if !found.contains(&line) {
                found.push(line); // the same person can be in two books
            }
        }
    }
    let _ = bus.close_sync(None::<&gio::Cancellable>);
    Ok(if found.is_empty() { format!("No contact matches “{name}”. Try part of the name, or another name you know them by.") } else { found.into_iter().take(8).collect::<Vec<_>>().join("\n") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        let sat = Day(2026, 11, 14);
        assert_eq!(time_range("tomorrow", sat).unwrap(), (Day(2026, 11, 15), 1, "tomorrow".into()));
        assert_eq!(time_range("", sat).unwrap().2, "today");
        let (d, n, _) = time_range("this week", sat).unwrap();
        assert_eq!((d, n), (sat, 2));
        let (d, n, _) = time_range("next week", sat).unwrap();
        assert_eq!((d, n), (Day(2026, 11, 16), 7));
        assert_eq!(time_range("tuesday", sat).unwrap().0, Day(2026, 11, 17));
        assert_eq!(time_range("saturday", sat).unwrap().0, sat);
        assert_eq!(time_range("next saturday", sat).unwrap().0, Day(2026, 11, 21));
        assert_eq!(time_range("2026-12-24", sat).unwrap().2, "Thursday 2026-12-24");
        assert!(time_range("someday", sat).is_err());
        assert!(time_range("2026-02-30", sat).is_err());
    }

    #[test]
    fn events() {
        let day = Day(2026, 11, 15).start();
        let events = vec![Event { summary: "Dentist".into(), start: day + 9 * 3600 + 1800, end: day + 10 * 3600 + 1800 },
                          Event { summary: "Holiday".into(), start: day, end: day + 86400 }];
        assert_eq!(format_events(events, "tomorrow"), "Sun 15 Nov, all day: Holiday\nSun 15 Nov, 09:30–10:30: Dentist");
        assert_eq!(format_events(vec![], "tomorrow"), "No events tomorrow.");
    }

    #[test]
    fn vcards() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Marek Nowak\r\nEMAIL;TYPE=WORK:marek@example.com\r\n\
TEL;X-EVOLUTION-E164=600100200,\"+48\";TYPE=CELL:+48 600 100 200\r\nORG:ACME;Sales\r\nNOTE:long \r\n note\r\nEND:VCARD";
        let c = parse_vcard(card);
        assert_eq!(format_contact(&c), "Marek Nowak · email marek@example.com · phone +48 600 100 200 (mobile) · ACME, Sales");
        assert_eq!(any_field_query("a\"b"), r#"(contains "x-evolution-any-field" "a\"b")"#);
    }
}

// -- writing: calendar events and email drafts ----------------------------------------

const SOURCES_BUS: &str = "org.gnome.evolution.dataserver.Sources5";
const CAL_IFACE_EDS: &str = "org.gnome.evolution.dataserver.Calendar";
/// The calendar factory's bus name carries EDS's API version; newest first.
const CAL_FACTORIES: [&str; 3] = ["org.gnome.evolution.dataserver.Calendar8", "org.gnome.evolution.dataserver.Calendar7",
                                  "org.gnome.evolution.dataserver.Calendar9"];

/// Calendars that take events: the user's default first, then the others
/// (not birthdays or weather), as (uid, name).
fn calendars(bus: &gio::DBusConnection) -> Result<Vec<(String, String)>, ToolError> {
    type Objects = BTreeMap<glib::variant::ObjectPath, BTreeMap<String, BTreeMap<String, glib::Variant>>>;
    let reply = bus.call_sync(Some(SOURCES_BUS), "/org/gnome/evolution/dataserver/SourceManager",
        "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None, Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()),
        gio::DBusCallFlags::NONE, 10000, None::<&gio::Cancellable>)
        .map_err(|_| ToolError("the calendar needs Evolution Data Server, which isn't running".into()))?;
    let (objects,): (Objects,) = reply.get().unwrap_or_default();
    let mut cals: Vec<(String, String)> = objects.values().filter_map(|ifaces| {
        let src = ifaces.get("org.gnome.evolution.dataserver.Source")?;
        let uid = src.get("UID")?.get::<String>()?;
        let data = src.get("Data")?.get::<String>()?;
        let enabled = !data.lines().any(|l| l.trim().eq_ignore_ascii_case("Enabled=false"));
        let skip = ["BackendName=contacts", "BackendName=weather"].iter().any(|b| data.contains(b));
        let name = data.lines().find_map(|l| l.strip_prefix("DisplayName=")).unwrap_or(&uid).to_string();
        (data.contains("[Calendar]") && enabled && !skip).then_some((uid, name))
    }).collect();
    // EDS keeps the default calendar here; check the key exists (GLib aborts on unknown keys).
    let default = gio::SettingsSchemaSource::default().and_then(|s| s.lookup("org.gnome.Evolution.DefaultSources", true))
        .filter(|schema| schema.has_key("default-calendar"))
        .map(|_| gio::Settings::new("org.gnome.Evolution.DefaultSources").string("default-calendar").to_string()).unwrap_or_default();
    cals.sort_by_key(|(uid, _)| (uid != &default, uid != "system-calendar"));
    if cals.is_empty() {
        return Err("no calendar to add to: set one up in GNOME Calendar or Online Accounts".into());
    }
    Ok(cals)
}

/// Open a calendar: (bus name, object path).
fn open_calendar(bus: &gio::DBusConnection, uid: &str) -> Result<(String, String), glib::Error> {
    let mut last = None;
    for factory in CAL_FACTORIES {
        match bus.call_sync(Some(factory), "/org/gnome/evolution/dataserver/CalendarFactory",
                            "org.gnome.evolution.dataserver.CalendarFactory", "OpenCalendar", Some(&(uid,).to_variant()),
                            Some(glib::VariantTy::new("(ss)").unwrap()), gio::DBusCallFlags::NONE, 15000, None::<&gio::Cancellable>) {
            Ok(v) => {
                let (path, name): (String, String) = v.get().unwrap_or_default();
                bus.call_sync(Some(&name), &path, CAL_IFACE_EDS, "Open", None, None, gio::DBusCallFlags::NONE, 15000,
                              None::<&gio::Cancellable>)?;
                return Ok((name, path));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

fn ics_time(t: i64) -> String {
    glib::DateTime::from_unix_utc(t).and_then(|d| d.format("%Y%m%dT%H%M%SZ")).map(|s| s.to_string()).unwrap_or_default()
}

fn ics_text(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

/// A VEVENT for EDS.
pub fn vevent(uid: &str, title: &str, start: i64, end: i64, place: &str, now: i64) -> String {
    let mut lines = vec!["BEGIN:VEVENT".to_string(), format!("UID:{uid}"), format!("DTSTAMP:{}", ics_time(now)),
                         format!("DTSTART:{}", ics_time(start)), format!("DTEND:{}", ics_time(end)),
                         format!("SUMMARY:{}", ics_text(title))];
    if !place.trim().is_empty() {
        lines.push(format!("LOCATION:{}", ics_text(place.trim())));
    }
    lines.push("END:VEVENT".into());
    lines.join("\r\n")
}

/// create_event: a title and a start (like reminders: HH:MM, a weekday, a date), an end or minutes.
pub fn create_event(ctx: &Context, args: &Args) -> ToolResult {
    let title = arg_str(args, "title");
    if title.trim().is_empty() {
        return Err("give the event a title".into());
    }
    let now = glib::DateTime::now_local().map_err(|_| ToolError("no local time".into()))?;
    let start_raw = arg_str(args, "start");
    let start = crate::reminders::parse_at(&start_raw, &now)
        .ok_or_else(|| ToolError(format!("couldn't read the start '{start_raw}': use HH:MM, a weekday and time, or YYYY-MM-DD HH:MM")))?
        .to_unix();
    let minutes = args.get("minutes").and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())).unwrap_or(60.0);
    let end = match arg_str(args, "end") {
        e if !e.trim().is_empty() => crate::reminders::parse_at(&e, &glib::DateTime::from_unix_local(start).unwrap_or(now))
            .map(|d| d.to_unix()).filter(|e| *e > start).unwrap_or(start + (minutes * 60.0) as i64),
        _ => start + (minutes * 60.0) as i64,
    };
    let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).map_err(|e| ToolError(e.message().into()))?;
    let uid = format!("{}-{}@panda-assistant", crate::reminders::now(), glib::random_int());
    let ics = vevent(&uid, &title, start, end, &arg_str(args, "place"), crate::reminders::now());
    let mut last = String::new();
    for (cal_uid, cal_name) in calendars(&bus)? {
        let Ok((name, path)) = open_calendar(&bus, &cal_uid) else { continue };
        match bus.call_sync(Some(&name), &path, CAL_IFACE_EDS, "CreateObjects", Some(&(vec![ics.clone()], 0u32).to_variant()),
                            None, gio::DBusCallFlags::NONE, 20000, None::<&gio::Cancellable>) {
            Ok(_) => {
                let when = glib::DateTime::from_unix_local(start).and_then(|d| d.format("%a %e %b, %H:%M"))
                    .map(|s| s.to_string().replace("  ", " ")).unwrap_or_default();
                let (n, p, u) = (name.clone(), path.clone(), uid.clone());
                ctx.can_undo(&format!("Add {} to the calendar", title.trim()), move || {
                    let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).map_err(|e| ToolError(e.message().into()))?;
                    // All occurrences ("all" is ECalObjModType's nick), no extra flags.
                    bus.call_sync(Some(&n), &p, CAL_IFACE_EDS, "RemoveObjects", Some(&(vec![(u.clone(), String::new())], "all", 0u32).to_variant()),
                                  None, gio::DBusCallFlags::NONE, 20000, None::<&gio::Cancellable>).map_err(|e| ToolError(e.message().into()))?;
                    Ok("the event is gone.".into())
                });
                return Ok(format!("Added “{}” to {cal_name}: {when}, until {}.", title.trim(),
                                  glib::DateTime::from_unix_local(end).and_then(|d| d.format("%H:%M")).map(|s| s.to_string()).unwrap_or_default()));
            }
            Err(e) => last = e.message().to_string(), // read-only calendar: try the next one
        }
    }
    Err(format!("couldn't add the event{}", if last.is_empty() { String::new() } else { format!(": {last}") }).into())
}

fn percent(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// draft_email: opens a ready-to-send draft in the user's mail app; they send it.
pub fn draft_email(_: &Context, args: &Args) -> ToolResult {
    let to_raw = arg_str(args, "to");
    let mut to = to_raw.trim().to_string();
    if !to.is_empty() && !to.contains('@') {
        // A name: the address from their contacts.
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).map_err(|e| ToolError(e.message().into()))?;
        let found: Vec<String> = address_books(&bus).unwrap_or_default().iter()
            .flat_map(|uid| query_book(&bus, uid, &any_field_query(&to)).unwrap_or_default())
            .flat_map(|card| parse_vcard(&card).emails).collect();
        to = found.first().cloned().ok_or_else(|| ToolError(format!("no email address for “{to_raw}” in your contacts; give the address")))?;
    }
    let mut query = vec![];
    for (k, v) in [("subject", arg_str(args, "subject")), ("body", arg_str(args, "body"))] {
        if !v.trim().is_empty() {
            query.push(format!("{k}={}", percent(v.trim())));
        }
    }
    let uri = format!("mailto:{}{}{}", percent(&to).replace("%40", "@"), if query.is_empty() { "" } else { "?" }, query.join("&"));
    gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        .map_err(|e| ToolError(format!("no mail app opened it: {e}")))?;
    Ok(format!("The draft is open in your mail app{}; check it and press Send.", if to.is_empty() { String::new() } else { format!(", to {to}") }))
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn events_and_mailto() {
        let ics = vevent("u1@x", "Lunch, with Ola", 1_795_000_000, 1_795_003_600, "Bistro; Kraków", 1_794_000_000);
        assert!(ics.starts_with("BEGIN:VEVENT\r\nUID:u1@x\r\n"));
        assert!(ics.contains("SUMMARY:Lunch\\, with Ola") && ics.contains("LOCATION:Bistro\\; Kraków"));
        assert!(ics.contains("DTSTART:20261118T") && ics.ends_with("END:VEVENT"));
        assert_eq!(percent("Late, sorry! 10 min"), "Late%2C%20sorry%21%2010%20min");
    }
}
