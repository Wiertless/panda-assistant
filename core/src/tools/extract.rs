//! Text out of the file types people keep, read directly so it works for any
//! file in home, indexed or not: office documents (Word, Writer, PowerPoint,
//! Impress, Excel, Calc), EPUB, RTF, HTML, e-mail, Jupyter notebooks, ZIP
//! archives (their file list), and any text or source file. PDFs go through
//! pdftotext. What isn't covered returns None and the caller falls back to
//! GNOME's index.

use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Most text taken from one file (a long novel is about 1 MB).
const MAX_CHARS: usize = 1_500_000;

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// The text of `path`, or None when it's a kind this can't read.
pub fn text(path: &Path) -> Option<Result<String, String>> {
    let e = ext(path);
    let out = match e.as_str() {
        "docx" | "docm" | "dotx" => zip_xml(path, &|n| n == "word/document.xml", Some("w:p")),
        "pptx" | "ppsx" => zip_xml(path, &|n| is_numbered(n, "ppt/slides/slide"), Some("a:p")),
        "xlsx" | "xlsm" => xlsx(path),
        "odt" | "ods" | "odp" | "odg" | "ott" => {
            zip_xml(path, &|n| n == "content.xml", Some("text:p"))
        }
        "epub" => zip_xml(
            path,
            &|n| {
                let n = n.to_lowercase();
                (n.ends_with(".xhtml") || n.ends_with(".html") || n.ends_with(".htm"))
                    && !n.contains("nav")
                    && !n.contains("toc")
            },
            Some("p"),
        ),
        "rtf" => read_string(path).map(|s| rtf(&s)),
        "html" | "htm" | "xhtml" => read_string(path).map(|s| super::web::page_text(&s).1),
        "eml" => read_string(path).map(|s| eml(&s)),
        "ipynb" => read_string(path).and_then(|s| notebook(&s)),
        "zip" | "jar" => zip_list(path),
        "pdf" => pdf(path),
        _ => {
            // Anything that is UTF-8 text (code, configs, logs, subtitles…).
            let mut head = vec![0; 4096];
            let n = std::fs::File::open(path)
                .and_then(|mut f| f.read(&mut head))
                .ok()?;
            if n == 0 {
                return Some(Ok(String::new()));
            }
            if head[..n].contains(&0) || std::str::from_utf8(&head[..n.min(4000)]).is_err() {
                return None;
            }
            read_string(path)
        }
    };
    Some(out.map(|t| cap(&t)))
}

fn cap(t: &str) -> String {
    if t.len() <= MAX_CHARS {
        return t.to_string();
    }
    let mut end = MAX_CHARS;
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[…]", &t[..end])
}

fn read_string(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// "ppt/slides/slide12.xml" for prefix "ppt/slides/slide".
fn is_numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|r| r.strip_suffix(".xml"))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn number_in(name: &str) -> u32 {
    name.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(f).map_err(|e| format!("not a valid {} file ({e})", ext(path)))
}

fn entry(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.by_ref().take(50_000_000).read_to_string(&mut s).ok()?;
    Some(s)
}

/// The text of the XML parts `want` picks (in order, slides by number), a
/// line per `para` element.
fn zip_xml(path: &Path, want: &dyn Fn(&str) -> bool, para: Option<&str>) -> Result<String, String> {
    let mut z = open_zip(path)?;
    let mut names: Vec<String> = z
        .file_names()
        .filter(|n| want(n))
        .map(String::from)
        .collect();
    if names.is_empty() {
        return Err("no text inside".into());
    }
    if ext(path) == "epub" {
        names = epub_order(&mut z)
            .filter(|o| !o.is_empty())
            .unwrap_or(names);
    } else {
        names.sort_by_key(|n| (number_in(n), n.clone()));
    }
    let mut out = String::new();
    for (i, n) in names.iter().enumerate() {
        let Some(xml) = entry(&mut z, n) else {
            continue;
        };
        if names.len() > 1 && ext(path).starts_with("ppt") {
            out.push_str(&format!("\n--- Slide {} ---\n", i + 1));
        }
        out.push_str(&xml_text(&xml, para));
        out.push('\n');
        if out.len() > MAX_CHARS {
            break;
        }
    }
    Ok(out)
}

/// The chapters of an EPUB in reading order (its OPF spine).
fn epub_order(z: &mut zip::ZipArchive<std::fs::File>) -> Option<Vec<String>> {
    static ROOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"full-path="([^"]+)""#).unwrap());
    static ITEM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<item\b[^>]*?\bid="([^"]+)"[^>]*?\bhref="([^"]+)""#).unwrap()
    });
    static ITEM2: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<item\b[^>]*?\bhref="([^"]+)"[^>]*?\bid="([^"]+)""#).unwrap()
    });
    static REF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<itemref\b[^>]*?\bidref="([^"]+)""#).unwrap());
    let container = entry(z, "META-INF/container.xml")?;
    let opf_path = ROOT.captures(&container)?[1].to_string();
    let opf = entry(z, &opf_path)?;
    let base = opf_path
        .rsplit_once('/')
        .map(|(b, _)| format!("{b}/"))
        .unwrap_or_default();
    let mut items = std::collections::HashMap::new();
    for c in ITEM.captures_iter(&opf) {
        items.insert(c[1].to_string(), c[2].to_string());
    }
    for c in ITEM2.captures_iter(&opf) {
        items.entry(c[2].to_string()).or_insert(c[1].to_string());
    }
    Some(
        REF.captures_iter(&opf)
            .filter_map(|c| items.get(&c[1]).map(|h| format!("{base}{h}")))
            .collect(),
    )
}

/// Plain text of XML: a newline at each paragraph end, tabs and breaks kept,
/// tags dropped, entities decoded.
fn xml_text(xml: &str, para: Option<&str>) -> String {
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let mut s = xml.to_string();
    if let Some(p) = para {
        s = s.replace(&format!("</{p}>"), "\n");
    }
    for (from, to) in [
        ("<w:tab/>", "\t"),
        ("<w:br/>", "\n"),
        ("<text:tab/>", "\t"),
        ("<text:line-break/>", "\n"),
        ("</text:h>", "\n"),
        ("</table:table-cell>", "\t"),
        ("</table:table-row>", "\n"),
        ("<br/>", "\n"),
        ("<br />", "\n"),
        ("</h1>", "\n"),
        ("</h2>", "\n"),
        ("</h3>", "\n"),
        ("</li>", "\n"),
        ("</div>", "\n"),
    ] {
        s = s.replace(from, to);
    }
    let s = TAG.replace_all(&s, "");
    let s = super::web::unescape(&s);
    s.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .split("\n\n\n")
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Spreadsheets: each sheet's rows as tab-separated cells.
fn xlsx(path: &Path) -> Result<String, String> {
    static SI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<si>(.*?)</si>").unwrap());
    static T: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<t[^>]*>(.*?)</t>").unwrap());
    static ROW: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)<row[^>]*>(.*?)</row>").unwrap());
    static CELL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?s)<c\b([^>]*?)(?:/>|>(.*?)</c>)"#).unwrap());
    static V: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<v>(.*?)</v>").unwrap());
    let mut z = open_zip(path)?;
    let shared: Vec<String> = entry(&mut z, "xl/sharedStrings.xml")
        .map(|x| {
            SI.captures_iter(&x)
                .map(|c| {
                    T.captures_iter(&c[1])
                        .map(|t| super::web::unescape(&t[1]))
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    let mut sheets: Vec<String> = z
        .file_names()
        .filter(|n| is_numbered(n, "xl/worksheets/sheet"))
        .map(String::from)
        .collect();
    sheets.sort_by_key(|n| number_in(n));
    let mut out = String::new();
    for (i, s) in sheets.iter().enumerate() {
        let Some(xml) = entry(&mut z, s) else {
            continue;
        };
        out.push_str(&format!("--- Sheet {} ---\n", i + 1));
        for row in ROW.captures_iter(&xml) {
            let cells: Vec<String> = CELL
                .captures_iter(&row[1])
                .map(|c| {
                    let attrs = &c[1];
                    let inner = c.get(2).map_or("", |m| m.as_str());
                    if attrs.contains(r#"t="s""#) {
                        V.captures(inner)
                            .and_then(|v| v[1].trim().parse::<usize>().ok())
                            .and_then(|i| shared.get(i).cloned())
                            .unwrap_or_default()
                    } else if attrs.contains(r#"t="inlineStr""#) {
                        T.captures_iter(inner)
                            .map(|t| super::web::unescape(&t[1]))
                            .collect()
                    } else {
                        V.captures(inner)
                            .map(|v| v[1].to_string())
                            .unwrap_or_default()
                    }
                })
                .collect();
            if cells.iter().any(|c| !c.is_empty()) {
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
            if out.len() > MAX_CHARS {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// RTF without its control words and groups like fonts and colours.
fn rtf(s: &str) -> String {
    static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\'([0-9a-fA-F]{2})").unwrap());
    static UNI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\u(-?\d+)\??").unwrap());
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\[a-zA-Z]+-?\d* ?").unwrap());
    let mut out = String::new();
    let mut depth_skip: Vec<i32> = vec![];
    let mut depth = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    // Drop the header groups (\fonttbl, \colortbl, \stylesheet, \info, \*…).
    let mut body = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            depth += 1;
            let rest: String = chars[i + 1..chars.len().min(i + 14)].iter().collect();
            if [
                "\\fonttbl",
                "\\colortbl",
                "\\stylesheet",
                "\\info",
                "\\*",
                "\\pict",
            ]
            .iter()
            .any(|k| rest.starts_with(k))
            {
                depth_skip.push(depth);
            }
        } else if c == '}' {
            if depth_skip.last() == Some(&depth) {
                depth_skip.pop();
            }
            depth -= 1;
        } else if depth_skip.is_empty() {
            body.push(c);
        }
        i += 1;
    }
    // A control word ends at a space, which belongs to it; \pard etc. are other words.
    static BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(par|line|tab)\b ?").unwrap());
    let body = BREAK.replace_all(&body, |c: &regex::Captures| {
        if &c[1] == "tab" { "\t" } else { "\n" }.to_string()
    });
    let body = UNI.replace_all(&body, |c: &regex::Captures| {
        c[1].parse::<i32>()
            .ok()
            .and_then(|n| char::from_u32(if n < 0 { (n + 65536) as u32 } else { n as u32 }))
            .map(String::from)
            .unwrap_or_default()
    });
    let body = HEX.replace_all(&body, |c: &regex::Captures| {
        // Windows-1250 covers Polish; the Latin-1 range is the same for ASCII letters.
        let b = u8::from_str_radix(&c[1], 16).unwrap_or(b'?');
        cp1250(b).to_string()
    });
    out.push_str(&WORD.replace_all(&body, ""));
    out.replace("\\\\", "\\")
        .replace("\\{", "{")
        .replace("\\}", "}")
}

fn cp1250(b: u8) -> char {
    const HIGH: [(u8, char); 18] = [
        (0xA5, 'Ą'),
        (0xB9, 'ą'),
        (0xC6, 'Ć'),
        (0xE6, 'ć'),
        (0xCA, 'Ę'),
        (0xEA, 'ę'),
        (0xA3, 'Ł'),
        (0xB3, 'ł'),
        (0xD1, 'Ń'),
        (0xF1, 'ń'),
        (0xD3, 'Ó'),
        (0xF3, 'ó'),
        (0x8C, 'Ś'),
        (0x9C, 'ś'),
        (0x8F, 'Ź'),
        (0x9F, 'ź'),
        (0xAF, 'Ż'),
        (0xBF, 'ż'),
    ];
    HIGH.iter()
        .find(|(k, _)| *k == b)
        .map(|(_, c)| *c)
        .unwrap_or(b as char)
}

/// An e-mail: its From/To/Date/Subject and the plain-text body.
fn eml(s: &str) -> String {
    let (head, body) = s
        .split_once("\r\n\r\n")
        .or_else(|| s.split_once("\n\n"))
        .unwrap_or((s, ""));
    let keep: Vec<&str> = head
        .lines()
        .filter(|l| {
            let l = l.to_lowercase();
            ["from:", "to:", "cc:", "date:", "subject:"]
                .iter()
                .any(|k| l.starts_with(k))
        })
        .collect();
    let body = if body.contains("<html") || body.contains("<HTML") {
        super::web::page_text(body).1
    } else {
        body.to_string()
    };
    format!("{}\n\n{}", keep.join("\n"), body.trim())
}

fn notebook(s: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(s).map_err(|e| e.to_string())?;
    let join = |x: &serde_json::Value| match x {
        serde_json::Value::Array(a) => a.iter().filter_map(|l| l.as_str()).collect::<String>(),
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    };
    Ok(v["cells"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let src = join(&c["source"]);
            if c["cell_type"] == "code" {
                format!("```\n{src}\n```")
            } else {
                src
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn zip_list(path: &Path) -> Result<String, String> {
    let z = open_zip(path)?;
    let names: Vec<&str> = z.file_names().take(500).collect();
    Ok(format!(
        "A ZIP archive with {} files:\n{}",
        z.len(),
        names.join("\n")
    ))
}

fn pdf(path: &Path) -> Result<String, String> {
    let out = super::run::output(
        &[
            "pdftotext",
            "-layout",
            "-l",
            "300",
            &path.to_string_lossy(),
            "-",
        ],
        std::time::Duration::from_secs(60),
        None,
    )
    .map_err(|e| e.0)?;
    if out.code != 0 {
        return Err(out.stderr.trim().to_string());
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, files: &[(&str, &str)]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (n, body) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn office_documents() {
        let d = tempfile::tempdir().unwrap();
        let docx = d.path().join("a.docx");
        zip_with(
            &docx,
            &[(
                "word/document.xml",
                r#"<w:document><w:body><w:p><w:r><w:t>Umowa najmu</w:t></w:r></w:p><w:p><w:r><w:t>Czynsz:</w:t><w:tab/><w:t>3 200 zł &amp; media</w:t></w:r></w:p></w:body></w:document>"#,
            )],
        );
        assert_eq!(
            text(&docx).unwrap().unwrap().trim(),
            "Umowa najmu\nCzynsz:\t3 200 zł & media"
        );

        let pptx = d.path().join("s.pptx");
        zip_with(
            &pptx,
            &[
                (
                    "ppt/slides/slide2.xml",
                    "<p:sld><a:p><a:t>Second</a:t></a:p></p:sld>",
                ),
                (
                    "ppt/slides/slide10.xml",
                    "<p:sld><a:p><a:t>Tenth</a:t></a:p></p:sld>",
                ),
                (
                    "ppt/slides/slide1.xml",
                    "<p:sld><a:p><a:t>First</a:t></a:p></p:sld>",
                ),
            ],
        );
        let t = text(&pptx).unwrap().unwrap();
        let (a, b, c) = (
            t.find("First").unwrap(),
            t.find("Second").unwrap(),
            t.find("Tenth").unwrap(),
        );
        assert!(a < b && b < c, "{t}");

        let xlsx = d.path().join("b.xlsx");
        zip_with(
            &xlsx,
            &[
                (
                    "xl/sharedStrings.xml",
                    "<sst><si><t>Item</t></si><si><t>Rent</t></si></sst>",
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>12</v></c></row><row r="2"><c t="s"><v>1</v></c><c><v>3200</v></c></row></sheetData></worksheet>"#,
                ),
            ],
        );
        assert_eq!(
            text(&xlsx).unwrap().unwrap(),
            "--- Sheet 1 ---\nItem\t12\nRent\t3200\n"
        );

        let odt = d.path().join("c.odt");
        zip_with(
            &odt,
            &[(
                "content.xml",
                "<office:text><text:h>Title</text:h><text:p>Hello <text:span>world</text:span></text:p></office:text>",
            )],
        );
        assert_eq!(text(&odt).unwrap().unwrap().trim(), "Title\nHello world");
    }

    #[test]
    fn epub_in_spine_order() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("book.epub");
        zip_with(
            &p,
            &[
                (
                    "META-INF/container.xml",
                    r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
                ),
                (
                    "OEBPS/content.opf",
                    r#"<package><manifest><item id="c2" href="ch2.xhtml"/><item id="c1" href="ch1.xhtml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
                ),
                (
                    "OEBPS/ch2.xhtml",
                    "<html><body><p>Chapter two</p></body></html>",
                ),
                (
                    "OEBPS/ch1.xhtml",
                    "<html><body><p>Chapter one</p></body></html>",
                ),
            ],
        );
        let t = text(&p).unwrap().unwrap();
        assert!(
            t.find("Chapter one").unwrap() < t.find("Chapter two").unwrap(),
            "{t}"
        );
    }

    #[test]
    fn rtf_eml_notebook_and_code() {
        let d = tempfile::tempdir().unwrap();
        let rtf_path = d.path().join("x.rtf");
        std::fs::write(&rtf_path, r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}\f0\fs24 Za\'bf\'f3\'b3\'e6 g\u281?\par Second line}").unwrap();
        assert_eq!(
            text(&rtf_path).unwrap().unwrap().trim(),
            "Zażółć gę\nSecond line"
        );

        let e = d.path().join("m.eml");
        std::fs::write(
            &e,
            "From: Ola <o@x.pl>\nX-Spam: 1\nSubject: Obiad\n\nW piątek o 13?\n",
        )
        .unwrap();
        assert_eq!(
            text(&e).unwrap().unwrap(),
            "From: Ola <o@x.pl>\nSubject: Obiad\n\nW piątek o 13?"
        );

        let nb = d.path().join("n.ipynb");
        std::fs::write(&nb, r##"{"cells":[{"cell_type":"markdown","source":["# Plan"]},{"cell_type":"code","source":["print(1)"]}]}"##).unwrap();
        assert_eq!(text(&nb).unwrap().unwrap(), "# Plan\n\n```\nprint(1)\n```");

        let code = d.path().join("main.rs");
        std::fs::write(&code, "fn main() {}\n").unwrap();
        assert_eq!(text(&code).unwrap().unwrap(), "fn main() {}\n");

        let bin = d.path().join("blob.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
        assert!(text(&bin).is_none());
    }
}
