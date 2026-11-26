//! What money from one year is worth in another, from the World Bank's consumer
//! price index (free, no key): only the country code leaves the computer.

use serde_json::Value;

use super::web;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

/// "5070", "5070pln", "5 070,50 zł" → 5070.5.
fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let kept: String = s
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect();
            kept.replace(',', ".").parse().ok()
        }
        _ => None,
    }
}

/// "98" → 1998, "05" → 2005, "1998" → 1998.
fn year(v: Option<&Value>, this_year: i64) -> Option<i64> {
    let y = number(v)? as i64;
    Some(match y {
        0..=99 if 2000 + y <= this_year => 2000 + y,
        0..=99 => 1900 + y,
        _ => y,
    })
}

/// The user's country from their locale (pl_PL → PL).
fn home_country() -> String {
    glib::language_names()
        .iter()
        .find_map(|l| {
            let region: String = l.split('_').nth(1)?.chars().take(2).collect();
            (region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic())).then_some(region)
        })
        .unwrap_or_else(|| "US".into())
}

/// {year: index} from the World Bank's reply to FP.CPI.TOTL.
fn index(reply: &Value) -> Vec<(i64, f64)> {
    let mut rows: Vec<(i64, f64)> = reply[1]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["date"].as_str()?.parse().ok()?, r["value"].as_f64()?)))
        .collect();
    rows.sort_by_key(|r| r.0);
    rows
}

fn answer(
    amount: f64,
    from: i64,
    to: Option<i64>,
    country: &str,
    rows: &[(i64, f64)],
) -> ToolResult {
    let Some(&(last, _)) = rows.last() else {
        return Err(format!("no price data for {country}").into());
    };
    let to = to.unwrap_or(last).min(last);
    let at = |y: i64| rows.iter().find(|r| r.0 == y).map(|r| r.1);
    let (Some(a), Some(b)) = (at(from), at(to)) else {
        return Err(format!("price data for {country} covers {}–{last}", rows[0].0).into());
    };
    let worth = amount * b / a;
    Ok(format!(
        "{amount} from {from} is worth about {:.0} in {to} prices ({country}: prices ×{:.2}, World Bank \
         consumer price index; {last} is the latest year it has).",
        worth,
        b / a
    ))
}

pub fn inflation(_: &Context, args: &Args) -> ToolResult {
    let amount = number(args.get("amount")).ok_or("amount must be a number")?;
    let this_year = glib::DateTime::now_local().map_or(2026, |d| d.year() as i64);
    let from =
        year(args.get("from_year"), this_year).ok_or("from_year must be a year, e.g. 1998")?;
    let to = year(args.get("to_year"), this_year);
    let raw = arg_str(args, "country");
    let country = match raw.trim() {
        "" => home_country(),
        c => c.to_uppercase(),
    };
    // It goes into the URL: letters only.
    if !(2..=3).contains(&country.len()) || !country.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err("country must be a two-letter code like PL or US".into());
    }
    let url = format!(
        "https://api.worldbank.org/v2/country/{country}/indicator/FP.CPI.TOTL?format=json&per_page=100"
    );
    let (body, _) = web::fetch(&url, None)?;
    let reply: Value = serde_json::from_str(&body)
        .map_err(|_| ToolError::from("the World Bank sent something unreadable"))?;
    answer(amount, from, to, &country, &index(&reply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn worth_in_another_year() {
        assert_eq!(number(Some(&json!("5 070,50 zł"))), Some(5070.5));
        assert_eq!(number(Some(&json!("5070pln"))), Some(5070.0));
        assert_eq!(year(Some(&json!(98)), 2026), Some(1998));
        assert_eq!(year(Some(&json!("05")), 2026), Some(2005));
        assert_eq!(year(Some(&json!("1998")), 2026), Some(1998));
        let reply = json!([{"page": 1}, [
            {"date": "2025", "value": 200.0},
            {"date": "2024", "value": null},
            {"date": "1998", "value": 75.0}
        ]]);
        let rows = index(&reply);
        assert_eq!(rows, [(1998, 75.0), (2025, 200.0)]);
        let out = answer(5070.0, 1998, None, "PL", &rows).unwrap();
        assert!(
            out.starts_with("5070 from 1998 is worth about 13520 in 2025 prices"),
            "{out}"
        );
        // "now" past the data means the latest year it has.
        assert!(
            answer(5070.0, 1998, Some(2026), "PL", &rows)
                .unwrap()
                .contains("in 2025 prices")
        );
        assert!(
            answer(1.0, 1950, None, "PL", &rows)
                .unwrap_err()
                .0
                .contains("1998–2025")
        );
    }
}
