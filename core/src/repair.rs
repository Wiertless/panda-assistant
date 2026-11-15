//! Fixing the small slips small models make in tool calls, using each tool's
//! own schema, so a call that means the right thing runs instead of failing:
//! "30%" for a number, "yes" for a boolean, "Next" or "skip" near an enum
//! value, a near-miss argument name, and JSON with single quotes, code
//! fences or trailing commas.

use serde_json::{Map, Value};

/// Parse arguments that aren't quite JSON.
pub fn parse_loose(raw: &str) -> Option<Map<String, Value>> {
    let t = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if t.is_empty() {
        return Some(Map::new());
    }
    let tries = [t.to_string(), without_trailing_commas(t), without_trailing_commas(&t.replace('\'', "\""))];
    tries.iter().find_map(|s| match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Some(m),
        // Arguments sent as a JSON string holding the object.
        Ok(Value::String(inner)) => serde_json::from_str::<Map<String, Value>>(&inner).ok(),
        _ => None,
    })
}

fn without_trailing_commas(s: &str) -> String {
    let re = regex::Regex::new(r",\s*([}\]])").unwrap();
    re.replace_all(s, "$1").to_string()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn fix_value(v: Value, spec: &Value) -> Value {
    if let Some(options) = spec["enum"].as_array() {
        let opts: Vec<&str> = options.iter().filter_map(Value::as_str).collect();
        if let Value::String(s) = &v {
            let low = s.trim().to_lowercase().replace([' ', '-'], "_");
            if opts.contains(&low.as_str()) {
                return Value::String(low);
            }
            // The one option it starts with, contains, or nearly spells.
            let close: Vec<&&str> = opts.iter()
                .filter(|o| o.starts_with(&low) || low.starts_with(**o) || edit_distance(o, &low) <= 1).collect();
            if close.len() == 1 {
                return Value::String(close[0].to_string());
            }
        }
        return v;
    }
    match (spec["type"].as_str(), &v) {
        (Some("integer" | "number"), Value::String(s)) => {
            let num: String = s.trim().trim_end_matches('%').trim().replace(',', ".").chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
            match num.parse::<f64>() {
                Ok(n) if spec["type"] == "integer" => Value::from(n.round() as i64),
                Ok(n) => serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(v),
                Err(_) => v,
            }
        }
        (Some("integer"), Value::Number(n)) if n.as_i64().is_none() => n.as_f64().map(|f| Value::from(f.round() as i64)).unwrap_or(v),
        (Some("boolean"), Value::String(s)) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "on" | "1" | "enable" | "enabled" | "tak" => Value::Bool(true),
            "false" | "no" | "off" | "0" | "disable" | "disabled" | "nie" => Value::Bool(false),
            _ => v,
        },
        (Some("boolean"), Value::Number(n)) => Value::Bool(n.as_f64() != Some(0.0)),
        (Some("string"), Value::Number(_) | Value::Bool(_)) => Value::String(v.to_string()),
        (Some("string"), Value::Array(a)) if a.iter().all(Value::is_string) =>
            Value::String(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
        _ => v,
    }
}

/// Arguments made to fit the tool's parameters schema. Returns what changed
/// (for logs and tests); unknown problems are left for the tool to report.
pub fn fit(schema: &Value, args: Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    let Some(props) = schema["properties"].as_object() else { return (args, vec![]) };
    let mut out = Map::new();
    let mut notes = vec![];
    let mut unknown = vec![];
    for (k, v) in args {
        let key = if props.contains_key(&k) { Some(k.clone()) } else {
            let low = k.to_lowercase().replace('-', "_");
            props.keys().find(|p| **p == low || edit_distance(p, &low) <= 2 || p.contains(&low) || low.contains(p.as_str())).cloned()
        };
        match key {
            Some(key) if !out.contains_key(&key) => {
                if key != k {
                    notes.push(format!("{k} -> {key}"));
                }
                let fixed = fix_value(v.clone(), &props[&key]);
                if fixed != v {
                    notes.push(format!("{key}: {v} -> {fixed}"));
                }
                out.insert(key, fixed);
            }
            _ => unknown.push((k, v)),
        }
    }
    // One required argument missing and one unknown given: that's the one.
    let missing: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str)
        .filter(|r| !out.contains_key(*r)).collect();
    if missing.len() == 1 && unknown.len() == 1 {
        let (k, v) = unknown.pop().unwrap();
        notes.push(format!("{k} -> {}", missing[0]));
        out.insert(missing[0].to_string(), fix_value(v, &props[missing[0]]));
    }
    for (k, v) in unknown {
        out.insert(k, v);
    }
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fitted(schema: Value, args: Value) -> Value {
        Value::Object(fit(&schema, args.as_object().unwrap().clone()).0)
    }

    #[test]
    fn values_fit_the_schema() {
        let vol = json!({"type": "object", "properties": {"percent": {"type": "integer"}}, "required": ["percent"]});
        assert_eq!(fitted(vol.clone(), json!({"percent": "30%"})), json!({"percent": 30}));
        assert_eq!(fitted(vol.clone(), json!({"volume": 40})), json!({"percent": 40}), "the one missing argument");
        assert_eq!(fitted(vol, json!({"percent": 42.6})), json!({"percent": 43}));
        let on = json!({"type": "object", "properties": {"enabled": {"type": "boolean"}}});
        assert_eq!(fitted(on.clone(), json!({"enabled": "yes"})), json!({"enabled": true}));
        assert_eq!(fitted(on, json!({"enable": "off"})), json!({"enabled": false}), "near-miss name");
        let media = json!({"type": "object", "properties": {"action": {"type": "string",
            "enum": ["play", "pause", "play_pause", "next", "previous", "stop", "status"]}}});
        assert_eq!(fitted(media.clone(), json!({"action": "Next"})), json!({"action": "next"}));
        assert_eq!(fitted(media.clone(), json!({"action": "prev"})), json!({"action": "previous"}));
        assert_eq!(fitted(media.clone(), json!({"action": "play pause"})), json!({"action": "play_pause"}));
        assert_eq!(fitted(media, json!({"action": "pl"})), json!({"action": "pl"}), "ambiguous: left for the tool");
        let text = json!({"type": "object", "properties": {"text": {"type": "string"}}});
        assert_eq!(fitted(text, json!({"text": ["milk", "eggs"]})), json!({"text": "milk, eggs"}));
    }

    #[test]
    fn loose_json() {
        assert_eq!(parse_loose(r#"{"a": 1,}"#).unwrap()["a"], 1);
        assert_eq!(parse_loose("{'device': 'phone'}").unwrap()["device"], "phone");
        assert_eq!(parse_loose("```json\n{\"x\": true}\n```").unwrap()["x"], true);
        assert_eq!(parse_loose(r#""{\"q\": \"hi\"}""#).unwrap()["q"], "hi");
        assert!(parse_loose("").unwrap().is_empty());
        assert!(parse_loose("not json").is_none());
    }
}
