//! Weather from Open-Meteo (free, no key): only the place name leaves the computer.

use serde_json::Value;

use super::web;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

/// WMO weather codes, as Open-Meteo reports them.
fn sky(code: i64) -> &'static str {
    match code {
        0 => "clear",
        1 => "mostly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51 | 53 | 55 => "drizzle",
        56 | 57 | 66 | 67 => "freezing rain",
        61 | 80 => "light rain",
        63 | 81 => "rain",
        65 | 82 => "heavy rain",
        71 | 85 => "light snow",
        73 | 75 | 86 => "snow",
        77 => "snow grains",
        95 => "thunderstorm",
        96 | 99 => "thunderstorm with hail",
        _ => "mixed",
    }
}

fn json(url: &str) -> Result<Value, ToolError> {
    let (body, _) = web::fetch(url, None)?;
    serde_json::from_str(&body)
        .map_err(|_| ToolError::from("the weather service sent something unreadable"))
}

/// "Toruń, Poland" with its forecast, from the reply of /v1/forecast.
pub fn format(place: &str, f: &Value) -> String {
    let c = &f["current"];
    let d = &f["daily"];
    let num = |v: &Value| v.as_f64().unwrap_or(0.0).round() as i64;
    let mut lines = vec![format!(
        "{place}: now {}°C (feels like {}°C), {}, wind {} km/h",
        num(&c["temperature_2m"]),
        num(&c["apparent_temperature"]),
        sky(c["weather_code"].as_i64().unwrap_or(-1)),
        num(&c["wind_speed_10m"]),
    )];
    let days = d["time"].as_array().map_or(0, Vec::len);
    for i in 0..days.min(3) {
        let label = match i {
            0 => "Today".to_string(),
            1 => "Tomorrow".to_string(),
            _ => d["time"][i].as_str().unwrap_or("").to_string(),
        };
        lines.push(format!(
            "{label}: {}…{}°C, {}, rain {}%",
            num(&d["temperature_2m_min"][i]),
            num(&d["temperature_2m_max"][i]),
            sky(d["weather_code"][i].as_i64().unwrap_or(-1)),
            num(&d["precipitation_probability_max"][i]),
        ));
    }
    lines.join("\n")
}

pub fn get_weather(_: &Context, args: &Args) -> ToolResult {
    let place = arg_str(args, "place");
    let place = place.trim();
    if place.is_empty() {
        return Err("say which city or town".into());
    }
    // Place names in the user's language ("Toruń", not "Torun").
    let lang: String = glib::language_names()
        .first()
        .map(|l| l.chars().take_while(char::is_ascii_alphabetic).collect())
        .filter(|l: &String| l.len() == 2)
        .unwrap_or_else(|| "en".into());
    let found = json(&format!(
        "https://geocoding-api.open-meteo.com/v1/search?count=1&format=json&language={lang}&name={}",
        web::url_encode(place)
    ))?;
    let hit = &found["results"][0];
    let (Some(lat), Some(lon)) = (hit["latitude"].as_f64(), hit["longitude"].as_f64()) else {
        return Err(ToolError::cannot(format!(
            "no place called “{place}” found"
        )));
    };
    let name = [hit["name"].as_str(), hit["country"].as_str()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let f = json(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&timezone=auto&forecast_days=3\
&current=temperature_2m,apparent_temperature,weather_code,wind_speed_10m\
&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max"
    ))?;
    Ok(format(&name, &f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn forecast_lines() {
        let f = json!({"current": {"temperature_2m": 4.4, "apparent_temperature": 0.6, "weather_code": 3, "wind_speed_10m": 12.2},
            "daily": {"time": ["2026-11-18", "2026-11-19", "2026-11-20"], "weather_code": [61, 0, 73],
                      "temperature_2m_min": [1.2, -2.0, -4.6], "temperature_2m_max": [6.0, 3.4, -0.5],
                      "precipitation_probability_max": [70, 5, 40]}});
        assert_eq!(
            format("Toruń, Poland", &f),
            "Toruń, Poland: now 4°C (feels like 1°C), overcast, wind 12 km/h\n\
Today: 1…6°C, light rain, rain 70%\nTomorrow: -2…3°C, clear, rain 5%\n2026-11-20: -5…-1°C, snow, rain 40%"
        );
    }
}
