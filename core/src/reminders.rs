//! Reminders and timers: kept in reminders.json, fired by the service as
//! notifications with Snooze and Done, and still there after a restart (the
//! service starts at login when any are pending, and fires missed ones).

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::paths;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

pub const SNOOZE_MINUTES: i64 = 10;
/// Fired reminders are kept this long for Snooze, then dropped.
const KEEP_FIRED: i64 = 24 * 3600;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reminder {
    pub id: String,
    /// Unix seconds.
    pub due: i64,
    pub text: String,
    #[serde(default)]
    pub fired: bool,
}

fn file() -> std::path::PathBuf {
    paths::data_dir().join("reminders.json")
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn load() -> Vec<Reminder> {
    fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(all: &[Reminder]) -> Result<(), ToolError> {
    let f = file();
    fs::create_dir_all(f.parent().unwrap()).map_err(|e| ToolError(e.to_string()))?;
    let tmp = f.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(all).unwrap()).map_err(|e| ToolError(e.to_string()))?;
    fs::rename(tmp, f).map_err(|e| ToolError(e.to_string()))
}

pub fn pending() -> Vec<Reminder> {
    let mut p: Vec<Reminder> = load().into_iter().filter(|r| !r.fired).collect();
    p.sort_by_key(|r| r.due);
    p
}

pub fn add(due: i64, text: &str) -> Result<Reminder, ToolError> {
    let mut all = load();
    let r = Reminder { id: format!("r{}{}", now(), all.len()), due, text: text.trim().to_string(), fired: false };
    all.push(r.clone());
    save(&all)?;
    Ok(r)
}

/// The reminders due now, marked fired (and old fired ones dropped).
pub fn take_due() -> Vec<Reminder> {
    let t = now();
    let mut all = load();
    let mut due = vec![];
    for r in all.iter_mut().filter(|r| !r.fired && r.due <= t) {
        r.fired = true;
        due.push(r.clone());
    }
    all.retain(|r| !(r.fired && r.due < t - KEEP_FIRED));
    if !due.is_empty() {
        let _ = save(&all);
    }
    due
}

pub fn snooze(id: &str, minutes: i64) -> Option<Reminder> {
    let mut all = load();
    let r = all.iter_mut().find(|r| r.id == id)?;
    r.due = now() + minutes * 60;
    r.fired = false;
    let r = r.clone();
    save(&all).ok()?;
    Some(r)
}

pub fn remove(id: &str) -> Option<Reminder> {
    let mut all = load();
    let i = all.iter().position(|r| r.id == id)?;
    let r = all.remove(i);
    save(&all).ok()?;
    Some(r)
}

/// "17:30", "2026-11-20 09:00", "2026-11-20T09:00", "tomorrow 9:00", "jutro 9:00"
/// in local time; a bare time already past today means tomorrow.
pub fn parse_at(text: &str, now: &glib::DateTime) -> Option<glib::DateTime> {
    static ISO_T: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(\d)t(\d)").unwrap());
    let t = ISO_T.replace_all(&text.trim().to_lowercase(), "$1 $2").replace("  ", " ");
    let (day_word, rest) = match t.split_once(' ') {
        Some((d, r)) if ["tomorrow", "jutro"].contains(&d) => (Some(1), r.to_string()),
        Some((d, r)) if ["today", "dziś", "dzis", "dzisiaj"].contains(&d) => (Some(0), r.to_string()),
        _ => (None, t.clone()),
    };
    let hm = |s: &str| -> Option<(i32, i32)> {
        let s = s.trim().trim_end_matches("am").trim_end_matches("pm").trim();
        let (h, m) = s.split_once([':', '.']).unwrap_or((s, "0"));
        let (mut h, m): (i32, i32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
        if text.to_lowercase().contains("pm") && h < 12 {
            h += 12;
        }
        (h < 24 && m < 60).then_some((h, m))
    };
    if let Some((date, time)) = rest.split_once(' ') && date.contains('-') {
        let mut d = date.split('-').map(|p| p.parse::<i32>().ok());
        let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
        let (h, m) = hm(time)?;
        return glib::DateTime::from_local(y, mo, da, h, m, 0.0).ok();
    }
    let (h, m) = hm(&rest)?;
    let today = glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), h, m, 0.0).ok()?;
    let plus = day_word.unwrap_or(if today.to_unix() <= now.to_unix() { 1 } else { 0 });
    today.add_days(plus).ok()
}

fn when(due: i64) -> String {
    let Ok(d) = glib::DateTime::from_unix_local(due) else { return String::new() };
    let today = glib::DateTime::now_local().ok().map(|n| (n.year(), n.day_of_year()));
    let fmt = if today == Some((d.year(), d.day_of_year())) { "%H:%M" } else { "%a %e %b, %H:%M" };
    d.format(fmt).map(|s| s.to_string().replace("  ", " ")).unwrap_or_default()
}

fn changed(ctx: &Context) {
    if let Some(f) = &ctx.reminders_changed {
        f();
    }
}

/// set_reminder: in_minutes or at; a timer is a reminder too.
pub fn set_reminder(ctx: &Context, args: &Args) -> ToolResult {
    let text = arg_str(args, "text");
    let text = if text.trim().is_empty() { "Timer".to_string() } else { text };
    let now_dt = glib::DateTime::now_local().map_err(|_| ToolError("no local time".into()))?;
    let minutes = args.get("in_minutes").and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()));
    let due = match (minutes, arg_str(args, "at")) {
        (Some(m), _) if m > 0.0 => now() + (m * 60.0).round() as i64,
        (_, at) if !at.trim().is_empty() => parse_at(&at, &now_dt)
            .ok_or_else(|| ToolError(format!("couldn't read the time '{at}': use HH:MM, YYYY-MM-DD HH:MM, or in_minutes")))?
            .to_unix(),
        _ => return Err("say when: in_minutes, or at (HH:MM or YYYY-MM-DD HH:MM)".into()),
    };
    if due <= now() {
        return Err("that time has passed".into());
    }
    let r = add(due, &text)?;
    changed(ctx);
    let mins = (due - now() + 30) / 60;
    let left = if mins < 60 { format!("in {mins} min") } else { format!("at {}", when(due)) };
    Ok(format!("Reminder set {left} ({}): {}", when(due), r.text))
}

pub fn list_reminders(_: &Context, _: &Args) -> ToolResult {
    let p = pending();
    Ok(if p.is_empty() { "No reminders.".into() } else {
        p.iter().map(|r| format!("{}: {}", when(r.due), r.text)).collect::<Vec<_>>().join("\n")
    })
}

pub fn cancel_reminder(ctx: &Context, args: &Args) -> ToolResult {
    let which = arg_str(args, "which");
    let p = pending();
    let names: Vec<String> = p.iter().map(|r| format!("{} {}", when(r.due), r.text)).collect();
    let pick = if p.len() == 1 && which.trim().is_empty() { names[0].clone() } else {
        crate::tools::pick(&which, &names, "reminder")?
    };
    let i = names.iter().position(|n| *n == pick).unwrap();
    remove(&p[i].id);
    changed(ctx);
    Ok(format!("Cancelled: {}", p[i].text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;
    use serde_json::json;

    fn args(v: serde_json::Value) -> Args {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn times() {
        let now = glib::DateTime::from_local(2026, 11, 15, 16, 0, 0.0).unwrap();
        let at = |s: &str| parse_at(s, &now).map(|d| d.format("%F %R").unwrap().to_string());
        assert_eq!(at("17:30").as_deref(), Some("2026-11-15 17:30"));
        assert_eq!(at("9:00").as_deref(), Some("2026-11-16 09:00"), "already past today");
        assert_eq!(at("tomorrow 9:00").as_deref(), Some("2026-11-16 09:00"));
        assert_eq!(at("jutro 9.15").as_deref(), Some("2026-11-16 09:15"));
        assert_eq!(at("2026-11-20 08:05").as_deref(), Some("2026-11-20 08:05"));
        assert_eq!(at("2026-11-20T08:05").as_deref(), Some("2026-11-20 08:05"));
        assert_eq!(at("5pm").as_deref(), Some("2026-11-15 17:00"));
        assert!(at("soon").is_none());
    }

    #[test]
    fn set_fire_snooze_cancel() {
        let _h = temp_home();
        let ctx = Context::default();
        let out = set_reminder(&ctx, &args(json!({"text": "take the pizza out", "in_minutes": 20}))).unwrap();
        assert!(out.contains("in 20 min") && out.contains("pizza"), "{out}");
        set_reminder(&ctx, &args(json!({"in_minutes": "5"}))).unwrap();
        assert!(set_reminder(&ctx, &args(json!({"text": "x"}))).is_err());
        let p = pending();
        assert_eq!((p.len(), p[0].text.as_str()), (2, "Timer"), "soonest first");
        // Due ones fire once.
        let mut all = load();
        all[0].due = now() - 1;
        save(&all).unwrap();
        let fired = take_due();
        assert_eq!(fired.len(), 1);
        assert!(take_due().is_empty());
        assert!(snooze(&fired[0].id, SNOOZE_MINUTES).is_some_and(|r| !r.fired && r.due > now()));
        assert_eq!(pending().len(), 2);
        assert!(cancel_reminder(&ctx, &args(json!({"which": "pizza"}))).unwrap().contains("pizza"));
        assert!(list_reminders(&ctx, &Args::new()).unwrap().contains("Timer"));
    }
}
