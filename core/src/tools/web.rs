//! Web search and page reading for the model.
//!
//! Search uses DuckDuckGo's lightweight HTML page (no API key) and falls back
//! to Wikipedia's search API. Results are kept short: local models run with
//! small contexts.

use std::io::Read;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use super::apps::check_url;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) Panda-Assistant";
const MAX_PAGE_BYTES: u64 = 2_000_000;
const MAX_RESULTS: usize = 5;
const SNIPPET_CHARS: usize = 220;
const PAGE_CHARS: usize = 3000;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .http_status_as_error(false)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

fn host(url: &str) -> String {
    url.split_once("://")
        .map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or("").to_string())
        .unwrap_or_default()
}

pub fn fetch(url: &str, form: Option<&str>) -> Result<(String, String), ToolError> {
    let a = agent();
    let resp = match form {
        Some(body) => a
            .post(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept-Language", "en-US,en;q=0.8")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes()),
        None => a
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept-Language", "en-US,en;q=0.8")
            .call(),
    }
    .map_err(|e| ToolError(format!("couldn't reach {}: {e}", host(url))))?;
    let status = resp.status().as_u16();
    if status >= 400 {
        return Err(format!("{url} returned HTTP {status}").into());
    }
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut bytes = vec![];
    resp.into_body()
        .into_reader()
        .take(MAX_PAGE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| ToolError(format!("couldn't read {}: {e}", host(url))))?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), ctype))
}

pub fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode HTML entities (the named ones pages actually use, and numeric ones).
pub fn unescape(text: &str) -> String {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);").unwrap());
    ENTITY
        .replace_all(text, |c: &regex::Captures| {
            let e = &c[1];
            let ch = if let Some(hex) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = e.strip_prefix('#') {
                dec.parse().ok().and_then(char::from_u32)
            } else {
                Some(match e {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    "nbsp" => ' ',
                    "ndash" => '–',
                    "mdash" => '—',
                    "hellip" => '…',
                    "rsquo" => '’',
                    "lsquo" => '‘',
                    "rdquo" => '”',
                    "ldquo" => '“',
                    "copy" => '©',
                    "oacute" => 'ó',
                    "Oacute" => 'Ó',
                    "eacute" => 'é',
                    "aacute" => 'á',
                    "uuml" => 'ü',
                    "ouml" => 'ö',
                    "auml" => 'ä',
                    "szlig" => 'ß',
                    "middot" => '·',
                    _ => return c[0].to_string(),
                })
            };
            ch.map(String::from).unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

fn clean(text: &str) -> String {
    unescape(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// HTML as a stream of tags and text, good enough for result pages.
enum Token<'a> {
    Open(String, &'a str),
    Close(String),
    Text(&'a str),
}

fn tokens(html: &str) -> Vec<Token<'_>> {
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<!--.*?-->|<(/?)([a-zA-Z][a-zA-Z0-9]*)([^>]*)>").unwrap()
    });
    let mut out = vec![];
    let mut last = 0;
    for c in TAG.captures_iter(html) {
        let m = c.get(0).unwrap();
        if m.start() > last {
            out.push(Token::Text(&html[last..m.start()]));
        }
        last = m.end();
        let Some(name) = c.get(2) else { continue }; // a comment
        let name = name.as_str().to_lowercase();
        if &c[1] == "/" {
            out.push(Token::Close(name));
        } else {
            out.push(Token::Open(name, c.get(3).map_or("", |a| a.as_str())));
        }
    }
    if last < html.len() {
        out.push(Token::Text(&html[last..]));
    }
    out
}

fn attr(attrs: &str, name: &str) -> String {
    let re = Regex::new(&format!(
        r#"(?i)\b{name}\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))"#
    ))
    .unwrap();
    re.captures(attrs)
        .map(|c| {
            c.get(2)
                .or(c.get(3))
                .or(c.get(4))
                .map_or("", |m| m.as_str())
                .to_string()
        })
        .map(|s| unescape(&s))
        .unwrap_or_default()
}

/// DuckDuckGo wraps result links as //duckduckgo.com/l/?uddg=<url>.
fn unwrap_link(href: &str) -> String {
    let href = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    if host(&href).ends_with("duckduckgo.com")
        && href.contains("/l/?")
        && let Some(q) = href.split_once('?').map(|(_, q)| q)
    {
        for pair in q.split('&') {
            if let Some(v) = pair.strip_prefix("uddg=") {
                return url_decode(v);
            }
        }
    }
    href
}

/// Results from lite.duckduckgo.com (result-link / result-snippet) and
/// html.duckduckgo.com (result__a / result__snippet).
pub fn parse_duckduckgo(html: &str) -> Vec<Hit> {
    let mut hits: Vec<Hit> = vec![];
    let mut field: Option<&str> = None;
    let mut buf = String::new();
    for tok in tokens(html) {
        match tok {
            Token::Open(tag, attrs) => {
                let class = attr(attrs, "class");
                if tag == "a" && (class.contains("result-link") || class.contains("result__a")) {
                    hits.push(Hit {
                        title: String::new(),
                        url: unwrap_link(&attr(attrs, "href")),
                        snippet: String::new(),
                    });
                    field = Some("title");
                    buf.clear();
                } else if (class.contains("result-snippet") || class.contains("result__snippet"))
                    && !hits.is_empty()
                {
                    field = Some("snippet");
                    buf.clear();
                }
            }
            Token::Close(tag) => {
                if let Some(f) = field.filter(|_| matches!(tag.as_str(), "a" | "td" | "div")) {
                    let last = hits.last_mut().unwrap();
                    if f == "title" {
                        last.title = clean(&buf)
                    } else {
                        last.snippet = clean(&buf)
                    }
                    field = None;
                }
            }
            Token::Text(t) => {
                if field.is_some() {
                    buf.push_str(t);
                }
            }
        }
    }
    hits.into_iter()
        .filter(|h| {
            !h.title.is_empty()
                && (h.url.starts_with("http://") || h.url.starts_with("https://"))
                && !h.url.contains("duckduckgo.com/y.js")
        })
        .collect()
}

fn search_wikipedia(query: &str) -> Result<Vec<Hit>, ToolError> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit={MAX_RESULTS}&format=json&utf8=1",
        url_encode(query)
    );
    let (body, _) = fetch(&url, None)?;
    let data: Value = serde_json::from_str(&body)
        .map_err(|_| ToolError("Wikipedia's answer wasn't JSON".into()))?;
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
    Ok(data["query"]["search"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| {
            let title = h["title"].as_str().unwrap_or("").to_string();
            Hit {
                url: format!(
                    "https://en.wikipedia.org/wiki/{}",
                    url_encode(&title.replace(' ', "_")).replace('+', "_")
                ),
                snippet: clean(&TAGS.replace_all(h["snippet"].as_str().unwrap_or(""), "")),
                title,
            }
        })
        .collect())
}

pub fn format_results(query: &str, source: &str, hits: &[Hit]) -> String {
    let mut lines = vec![format!("Results from {source} for '{query}':")];
    for (i, h) in hits.iter().take(MAX_RESULTS).enumerate() {
        let snippet: String = h.snippet.chars().take(SNIPPET_CHARS).collect();
        lines.push(format!(
            "{}. {} - {}{}",
            i + 1,
            h.title,
            h.url,
            if snippet.is_empty() {
                String::new()
            } else {
                format!("\n   {snippet}")
            }
        ));
    }
    lines.push("Use read_web_page on a result for details.".into());
    lines.join("\n")
}

/// Search results and where they came from: DuckDuckGo, or Wikipedia when it has none.
pub fn search(query: &str) -> Result<(&'static str, Vec<Hit>), ToolError> {
    let query = query.trim();
    if query.is_empty() {
        return Err("empty search".into());
    }
    let mut errors = vec![];
    let ddg = fetch(
        "https://lite.duckduckgo.com/lite/",
        Some(&format!("q={}", url_encode(query))),
    )
    .map(|(b, _)| parse_duckduckgo(&b));
    match ddg {
        Ok(hits) if !hits.is_empty() => return Ok(("DuckDuckGo", hits)),
        Ok(_) => {}
        Err(e) => errors.push(e.0),
    }
    match search_wikipedia(query) {
        Ok(hits) if !hits.is_empty() => Ok(("Wikipedia", hits)),
        Ok(_) => Err(format!(
            "no results; try fewer or different words{}",
            if errors.is_empty() {
                String::new()
            } else {
                format!(" ({})", errors.join("; "))
            }
        )
        .into()),
        Err(e) => {
            errors.push(e.0);
            Err(format!("no results ({})", errors.join("; ")).into())
        }
    }
}

pub fn web_search(ctx: &Context, args: &Args) -> ToolResult {
    let query = arg_str(args, "query").trim().to_string();
    // "Quick research on…": small models take "quick" for one search; the user asked for research.
    let request = ctx.request.lock().unwrap().clone();
    if super::research::asks_research(&request) {
        let mut a = Args::new();
        a.insert(
            "question".into(),
            serde_json::Value::String(if query.is_empty() { request } else { query }),
        );
        return super::research::research(ctx, &a);
    }
    let (source, hits) = search(&query)?;
    Ok(format_results(&query, source, &hits))
}

/// (title, readable text) of an HTML page.
pub fn page_text(html: &str) -> (String, String) {
    const SKIP: [&str; 10] = [
        "script", "style", "noscript", "nav", "header", "footer", "aside", "form", "svg",
        "template",
    ];
    const BLOCK: [&str; 14] = [
        "p", "div", "li", "br", "h1", "h2", "h3", "h4", "tr", "section", "article", "pre", "td",
        "ul",
    ];
    let (mut title, mut text) = (String::new(), String::new());
    let (mut skip, mut in_title) = (0usize, false);
    for tok in tokens(html) {
        match tok {
            Token::Open(tag, _) if SKIP.contains(&tag.as_str()) => skip += 1,
            Token::Open(tag, _) if tag == "title" => in_title = true,
            Token::Open(tag, _) if BLOCK.contains(&tag.as_str()) => text.push('\n'),
            Token::Close(tag) if SKIP.contains(&tag.as_str()) => skip = skip.saturating_sub(1),
            Token::Close(tag) if tag == "title" => in_title = false,
            Token::Text(t) if in_title => title.push_str(t),
            Token::Text(t) if skip == 0 => text.push_str(t),
            _ => {}
        }
    }
    let lines: Vec<String> = text
        .split('\n')
        .map(clean)
        .filter(|l| l.chars().count() > 1)
        .collect();
    (clean(&title), lines.join("\n"))
}

/// (title, readable text) of a web page, cut to about `max_chars`.
pub fn read_page(url: &str, max_chars: usize) -> Result<(String, String), ToolError> {
    let url = check_url(url)?;
    let (body, ctype) = fetch(&url, None)?;
    let head = body
        .trim_start()
        .chars()
        .take(15)
        .collect::<String>()
        .to_lowercase();
    let (title, mut text) =
        if ctype.contains("html") || head.starts_with("<!doctype") || head.starts_with("<html") {
            page_text(&body)
        } else if ctype.starts_with("text/") || ctype.starts_with("application/json") {
            (String::new(), body)
        } else {
            return Err(format!(
                "can't read {}",
                if ctype.is_empty() {
                    "this kind of file"
                } else {
                    &ctype
                }
            )
            .into());
        };
    if text.chars().count() > max_chars {
        let cut: String = text.chars().take(max_chars).collect();
        text = format!(
            "{} […]",
            cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a)
        );
    }
    Ok((title, text))
}

pub fn read_web_page(_: &Context, args: &Args) -> ToolResult {
    let url = check_url(&arg_str(args, "url"))?;
    let (title, text) = read_page(&url, PAGE_CHARS)?;
    let head = if title.is_empty() {
        format!("{url}\n\n")
    } else {
        format!("{title}\n{url}\n\n")
    };
    Ok(head
        + if text.is_empty() {
            "(no readable text)"
        } else {
            &text
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duckduckgo_lite_and_html() {
        let lite = r#"<table><tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ffedoraproject.org%2Fnews&amp;rut=x" class='result-link'>Fedora &amp; News</a></td></tr>
<tr><td class='result-snippet'>Fedora 45 is <b>out</b>.</td></tr>
<a class="result-link" href="https://duckduckgo.com/y.js?ad=1">Ad</a></table>"#;
        let hits = parse_duckduckgo(lite);
        assert_eq!(
            hits,
            vec![Hit {
                title: "Fedora & News".into(),
                url: "https://fedoraproject.org/news".into(),
                snippet: "Fedora 45 is out.".into()
            }]
        );
        let html = r#"<a class="result__a" href="https://a.example/">A</a><a class="result__snippet">Snip</a>"#;
        assert_eq!(parse_duckduckgo(html)[0].snippet, "Snip");
    }

    #[test]
    fn page_text_skips_chrome() {
        let (title, text) = page_text(
            "<html><head><title> Hi &ndash; there</title><style>x{}</style></head>\
<body><nav>Menu</nav><h1>Head</h1><p>First   para.</p><script>var a=1;</script><p>Second</p><footer>f</footer></body></html>",
        );
        assert_eq!(title, "Hi – there");
        assert_eq!(text, "Head\nFirst para.\nSecond");
    }

    #[test]
    fn encoding() {
        assert_eq!(url_encode("a b&c/ł"), "a+b%26c%2F%C5%82");
        assert_eq!(
            url_decode("https%3A%2F%2Fx.pl%2F%C5%82+y"),
            "https://x.pl/ł y"
        );
        assert_eq!(unescape("&#39;&#x41;&amp;&unknown;"), "'A&&unknown;");
        let out = format_results(
            "q",
            "DuckDuckGo",
            &[Hit {
                title: "T".into(),
                url: "https://u".into(),
                snippet: "s".into(),
            }],
        );
        assert_eq!(
            out,
            "Results from DuckDuckGo for 'q':\n1. T - https://u\n   s\nUse read_web_page on a result for details."
        );
    }
}
