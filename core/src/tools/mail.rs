//! Mail from the accounts in GNOME Online Accounts, read over IMAP, so it works
//! whatever mail app the user has (Convey, Thunderbird, Evolution, Geary, webmail).
//! Read-only: mailboxes are opened with EXAMINE and bodies fetched with PEEK, so
//! nothing is marked as read. Credentials come from GOA and stay in this process.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use base64::Engine;
use regex::Regex;

use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const GOA: &str = "org.gnome.OnlineAccounts";
const PER_ACCOUNT: usize = 5;
const SNIPPET: usize = 300;

struct Account {
    path: String,
    email: String,
    host: String,
    user: String,
    tls: bool,
    oauth: bool,
}

/// A dictionary's value by its string key (a{sv} values come out of their variant box).
fn lookup(dict: &glib::Variant, key: &str) -> Option<glib::Variant> {
    (0..dict.n_children())
        .map(|i| dict.child_value(i))
        .find(|e| e.child_value(0).str() == Some(key))
        .map(|e| {
            let v = e.child_value(1);
            if v.type_().is_variant() {
                v.as_variant().unwrap_or(v)
            } else {
                v
            }
        })
}

fn accounts() -> Result<Vec<Account>, ToolError> {
    let none = || ToolError::cannot("no mail accounts: add one in Settings → Online Accounts");
    let all = super::run::dbus_call(
        false,
        GOA,
        "/org/gnome/OnlineAccounts",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        None,
        None,
        5000,
    )
    .map_err(|_| none())?;
    let objects = all.child_value(0);
    let mut out = vec![];
    for i in 0..objects.n_children() {
        let entry = objects.child_value(i);
        let path = entry.child_value(0).str().unwrap_or_default().to_string();
        let ifaces = entry.child_value(1);
        let Some(mail) = lookup(&ifaces, "org.gnome.OnlineAccounts.Mail") else {
            continue;
        };
        let s = |k: &str| {
            lookup(&mail, k)
                .and_then(|v| v.get::<String>())
                .unwrap_or_default()
        };
        let b = |k: &str| {
            lookup(&mail, k)
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false)
        };
        let disabled = lookup(&ifaces, "org.gnome.OnlineAccounts.Account")
            .and_then(|a| lookup(&a, "MailDisabled"))
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false);
        if disabled || s("ImapHost").is_empty() {
            continue;
        }
        out.push(Account {
            oauth: lookup(&ifaces, "org.gnome.OnlineAccounts.OAuth2Based").is_some(),
            path,
            email: s("EmailAddress"),
            host: s("ImapHost"),
            user: s("ImapUserName"),
            tls: b("ImapUseSsl"),
        });
    }
    if out.is_empty() {
        return Err(none());
    }
    Ok(out)
}

fn secret(a: &Account) -> Result<String, ToolError> {
    let (iface, method, args) = if a.oauth {
        (
            "org.gnome.OnlineAccounts.OAuth2Based",
            "GetAccessToken",
            None,
        )
    } else {
        (
            "org.gnome.OnlineAccounts.PasswordBased",
            "GetPassword",
            Some(glib::Variant::from(("imap-password",))),
        )
    };
    let r = super::run::dbus_call(
        false,
        GOA,
        &a.path,
        iface,
        method,
        args.as_ref(),
        None,
        20000,
    )
    .map_err(|e| {
        ToolError(format!(
            "{}: couldn't get its sign-in from Online Accounts ({e})",
            a.email
        ))
    })?;
    Ok(r.child_value(0).get::<String>().unwrap_or_default())
}

trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

struct Imap {
    r: BufReader<Box<dyn Stream>>,
    tag: u32,
}

/// One response line, and the literal ({n} bytes) that came with it, if any.
type Item = (String, Vec<u8>);

impl Imap {
    fn connect(a: &Account) -> Result<Self, ToolError> {
        let err = |e: &dyn std::fmt::Display| ToolError(format!("{}: {e}", a.host));
        let port = if a.tls { 993 } else { 143 };
        let addr = (a.host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| err(&e))?
            .next()
            .ok_or_else(|| err(&"no address"))?;
        let tcp =
            TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| err(&e))?;
        tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
        if !a.tls {
            // shortcut: plain IMAP (STARTTLS) isn't spoken, only 993 with TLS; add STARTTLS if an account needs it.
            return Err(ToolError::cannot(format!(
                "{}: only IMAP over TLS (port 993) is supported",
                a.email
            )));
        }
        use rustls_platform_verifier::ConfigVerifierExt;
        let config = rustls::ClientConfig::with_platform_verifier().map_err(|e| err(&e))?;
        let name = rustls::pki_types::ServerName::try_from(a.host.clone()).map_err(|e| err(&e))?;
        let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| err(&e))?;
        let stream: Box<dyn Stream> = Box::new(rustls::StreamOwned::new(conn, tcp));
        let mut imap = Imap {
            r: BufReader::new(stream),
            tag: 0,
        };
        imap.line().map_err(|e| err(&e))?; // greeting
        Ok(imap)
    }

    fn line(&mut self) -> std::io::Result<String> {
        let mut l = String::new();
        if self.r.read_line(&mut l)? == 0 {
            return Err(std::io::Error::other("the server closed the connection"));
        }
        Ok(l)
    }

    /// Send a command; the untagged responses until its tagged OK.
    fn cmd(&mut self, c: &str) -> Result<Vec<Item>, ToolError> {
        static LITERAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\d+)\}\r?\n$").unwrap());
        self.tag += 1;
        let tag = format!("A{}", self.tag);
        let io = |e: std::io::Error| ToolError(e.to_string());
        self.r
            .get_mut()
            .write_all(format!("{tag} {c}\r\n").as_bytes())
            .map_err(io)?;
        self.r.get_mut().flush().map_err(io)?;
        let mut out = vec![];
        loop {
            let l = self.line().map_err(io)?;
            if let Some(rest) = l.strip_prefix(&format!("{tag} ")) {
                return if rest.starts_with("OK") {
                    Ok(out)
                } else {
                    Err(ToolError(rest.trim().to_string()))
                };
            }
            let mut lit = vec![];
            if let Some(n) = LITERAL
                .captures(&l)
                .and_then(|m| m[1].parse::<usize>().ok())
            {
                lit = vec![0; n.min(10_000_000)];
                self.r.read_exact(&mut lit).map_err(io)?;
            }
            out.push((l, lit));
        }
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn login(a: &Account) -> Result<Imap, ToolError> {
    let mut imap = Imap::connect(a)?;
    let secret = secret(a)?;
    let auth = if a.oauth {
        let sasl = format!("user={}\x01auth=Bearer {secret}\x01\x01", a.user);
        imap.cmd(&format!(
            "AUTHENTICATE XOAUTH2 {}",
            base64::engine::general_purpose::STANDARD.encode(sasl)
        ))
    } else {
        imap.cmd(&format!("LOGIN {} {}", quote(&a.user), quote(&secret)))
    };
    auth.map_err(|e| ToolError(format!("{}: sign-in refused ({})", a.email, e.0)))?;
    Ok(imap)
}

/// A message: its headers' From, Subject and Date, and the text of its body.
#[derive(Debug, Default, PartialEq)]
pub struct Mail {
    pub uid: u32,
    pub from: String,
    pub subject: String,
    pub date: String,
    pub text: String,
}

/// Messages out of a UID FETCH of header fields and the start of the body.
fn parse_fetch(items: &[Item]) -> Vec<Mail> {
    static UID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"UID (\d+)").unwrap());
    let mut out: Vec<Mail> = vec![];
    let mut head = String::new();
    for (line, lit) in items {
        if line.contains(" FETCH (") {
            head.clear();
            out.push(Mail {
                uid: UID
                    .captures(line)
                    .and_then(|c| c[1].parse().ok())
                    .unwrap_or(0),
                ..Mail::default()
            });
        }
        let Some(m) = out.last_mut() else { continue };
        if m.uid == 0
            && let Some(c) = UID.captures(line)
        {
            m.uid = c[1].parse().unwrap_or(0);
        }
        if line.contains("HEADER.FIELDS") {
            head = String::from_utf8_lossy(lit).into_owned();
            m.from = decode_words(&header(&head, "from"));
            m.subject = decode_words(&header(&head, "subject"));
            m.date = header(&head, "date");
        } else if line.contains("BODY[TEXT]") {
            m.text = body_text(&head, lit);
        }
    }
    out
}

/// One header's value, with folded lines joined.
fn header(head: &str, name: &str) -> String {
    let mut value = String::new();
    let mut on = false;
    for l in head.lines() {
        if l.starts_with([' ', '\t']) {
            if on {
                value.push(' ');
                value.push_str(l.trim());
            }
            continue;
        }
        on = l
            .split_once(':')
            .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(name));
        if on {
            value = l
                .split_once(':')
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default();
        }
    }
    value
}

/// RFC 2047 words: =?utf-8?B?…?= and =?iso-8859-2?Q?…?=.
fn decode_words(s: &str) -> String {
    static WORD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"=\?([^?]+)\?([bBqQ])\?([^?]*)\?=").unwrap());
    let joined = Regex::new(r"\?=\s+=\?").unwrap().replace_all(s, "?==?");
    WORD.replace_all(&joined, |c: &regex::Captures| {
        let bytes = if c[2].eq_ignore_ascii_case("b") {
            base64::engine::general_purpose::STANDARD
                .decode(c[3].trim())
                .unwrap_or_default()
        } else {
            quoted_printable(&c[3].replace('_', " "))
        };
        charset(&c[1], &bytes)
    })
    .into_owned()
}

fn charset(name: &str, bytes: &[u8]) -> String {
    let n = name.to_lowercase();
    if n.contains("8859-2") || n.contains("1250") {
        // The Polish letters; the rest is ASCII.
        const PL: [(u8, char); 18] = [
            (0xA1, 'Ą'),
            (0xB1, 'ą'),
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
            (0xA6, 'Ś'),
            (0xB6, 'ś'),
            (0xAC, 'Ź'),
            (0xBC, 'ź'),
            (0xAF, 'Ż'),
            (0xBF, 'ż'),
        ];
        return bytes
            .iter()
            .map(|b| {
                PL.iter()
                    .find(|(k, _)| k == b)
                    .map(|(_, c)| *c)
                    .unwrap_or(*b as char)
            })
            .collect();
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn quoted_printable(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if b.get(i + 1) == Some(&b'\r') || b.get(i + 1) == Some(&b'\n') {
                i += if b.get(i + 1) == Some(&b'\r') { 3 } else { 2 };
                continue;
            }
            if let Some(h) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(h);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// The readable text of a message body: the first text/plain part, else HTML as
/// text, decoded from base64 or quoted-printable.
fn body_text(head: &str, body: &[u8]) -> String {
    static BOUNDARY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)boundary="?([^";\s]+)"?"#).unwrap());
    let ctype = header(head, "content-type").to_lowercase();
    let raw = String::from_utf8_lossy(body);
    // Without a Content-Type header a body that opens with "--boundary" is still multipart.
    let bare = raw
        .trim_start()
        .strip_prefix("--")
        .and_then(|r| r.lines().next())
        .map(|b| b.trim().to_string())
        .filter(|b| !ctype.starts_with("multipart/") && !b.is_empty() && !b.contains(' '));
    if ctype.starts_with("multipart/") || bare.is_some() {
        let Some(b) = BOUNDARY
            .captures(&header(head, "content-type"))
            .map(|c| c[1].to_string())
            .or(bare)
        else {
            return raw.into_owned();
        };
        let parts: Vec<(String, String)> = raw
            .split(&format!("--{b}"))
            .skip(1)
            .filter_map(|p| {
                let p = p.trim_start_matches(['\r', '\n']);
                let (h, rest) = p.split_once("\r\n\r\n").or_else(|| p.split_once("\n\n"))?;
                Some((h.to_string(), rest.to_string()))
            })
            .collect();
        let pick = |want: &str| {
            parts.iter().find(|(h, _)| {
                let t = header(h, "content-type").to_lowercase();
                t.starts_with(want) || (want == "multipart/" && t.starts_with("multipart/"))
            })
        };
        if let Some((h, b)) = pick("text/plain")
            .or_else(|| pick("multipart/"))
            .or_else(|| pick("text/html"))
        {
            return body_text(h, b.as_bytes());
        }
        return String::new();
    }
    let enc = header(head, "content-transfer-encoding").to_lowercase();
    let bytes = if enc.contains("base64") {
        let clean: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        let usable = clean.len() / 4 * 4; // the fetch may cut it short
        base64::engine::general_purpose::STANDARD
            .decode(&clean[..usable])
            .unwrap_or_default()
    } else if enc.contains("quoted-printable") {
        quoted_printable(&raw)
    } else {
        body.to_vec()
    };
    let cs = BOUNDARY_CHARSET
        .captures(&ctype)
        .map(|c| c[1].to_string())
        .unwrap_or_default();
    let text = charset(&cs, &bytes);
    if ctype.starts_with("text/html") {
        super::web::page_text(&text).1
    } else {
        super::web::unescape(&text)
    }
}

static BOUNDARY_CHARSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"charset="?([^";\s]+)"?"#).unwrap());

const FIELDS: &str =
    "BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE CONTENT-TYPE CONTENT-TRANSFER-ENCODING)]";

fn fetch(imap: &mut Imap, uids: &[u32], bytes: usize) -> Result<Vec<Mail>, ToolError> {
    let set: Vec<String> = uids.iter().map(u32::to_string).collect();
    let items = imap.cmd(&format!(
        "UID FETCH {} (UID {FIELDS} BODY.PEEK[TEXT]<0.{bytes}>)",
        set.join(",")
    ))?;
    Ok(parse_fetch(&items))
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        format!("{}…", flat.chars().take(max).collect::<String>())
    } else {
        flat
    }
}

/// IMAP TEXT terms, one per word, all required, cut to a stem so Polish endings
/// match ("algebra" finds "algebry"): TEXT matches substrings.
fn text_search(query: &str) -> String {
    query
        .split_whitespace()
        .map(|w| {
            let n = w.chars().count();
            let keep = if n > 6 {
                n - 2
            } else if n > 4 {
                n - 1
            } else {
                n
            };
            format!("TEXT {}", quote(&w.chars().take(keep).collect::<String>()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn search_mail(_: &Context, args: &Args) -> ToolResult {
    // Only words go into the IMAP command: no quotes, backslashes or line breaks.
    let query: String = arg_str(args, "query")
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .collect();
    let query = query.trim();
    let mut found = vec![];
    let mut errors = vec![];
    for a in accounts()? {
        let got = (|| -> Result<Vec<Mail>, ToolError> {
            let mut imap = login(&a)?;
            imap.cmd("EXAMINE INBOX")?;
            let search = if query.is_empty() {
                "UID SEARCH ALL".to_string()
            } else if a.host.ends_with("gmail.com") {
                format!("UID SEARCH X-GM-RAW {}", quote(query))
            } else {
                format!("UID SEARCH {}", text_search(query))
            };
            let uids: Vec<u32> = imap
                .cmd(&search)?
                .iter()
                .filter_map(|(l, _)| l.strip_prefix("* SEARCH"))
                .flat_map(|l| {
                    l.split_whitespace()
                        .filter_map(|n| n.parse().ok())
                        .collect::<Vec<u32>>()
                })
                .collect();
            let newest: Vec<u32> = uids.iter().rev().take(PER_ACCOUNT).copied().collect();
            if newest.is_empty() {
                return Ok(vec![]);
            }
            let mut mails = fetch(&mut imap, &newest, 3000)?;
            mails.sort_by_key(|m| std::cmp::Reverse(m.uid));
            let _ = imap.cmd("LOGOUT");
            Ok(mails)
        })();
        match got {
            Ok(m) => found.extend(m.into_iter().map(|m| (a.email.clone(), m))),
            Err(e) => errors.push(e.0),
        }
    }
    if found.is_empty() {
        let tail = if errors.is_empty() {
            String::new()
        } else {
            format!(" ({})", errors.join("; "))
        };
        return Ok(format!("No mail matches “{query}”{tail}."));
    }
    let lines: Vec<String> = found
        .iter()
        .map(|(acct, m)| {
            format!(
                "[{acct}:{}] {} — {} ({})\n  {}",
                m.uid,
                m.from,
                m.subject,
                m.date,
                one_line(&m.text, SNIPPET)
            )
        })
        .collect();
    Ok(format!(
        "Mail matching “{query}”, newest first:\n{}\nread_mail with an id in brackets reads the whole message.",
        lines.join("\n")
    ))
}

pub fn read_mail(_: &Context, args: &Args) -> ToolResult {
    let id = arg_str(args, "id");
    let id = id.trim().trim_matches(['[', ']']);
    let (acct, uid) = id
        .rsplit_once(':')
        .and_then(|(a, u)| Some((a.to_string(), u.parse::<u32>().ok()?)))
        .ok_or_else(|| {
            ToolError::from("give the id search_mail showed, like [name@example.com:1234]")
        })?;
    let a = accounts()?
        .into_iter()
        .find(|a| a.email.eq_ignore_ascii_case(&acct))
        .ok_or_else(|| ToolError(format!("no mail account {acct}")))?;
    let mut imap = login(&a)?;
    imap.cmd("EXAMINE INBOX")?;
    let m = fetch(&mut imap, &[uid], 60_000)?
        .pop()
        .ok_or_else(|| ToolError::cannot("that message isn't in the inbox any more"))?;
    let _ = imap.cmd("LOGOUT");
    let text: String = m.text.trim().chars().take(12_000).collect();
    Ok(format!(
        "From: {}\nSubject: {}\nDate: {}\n\n{text}",
        m.from, m.subject, m.date
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_responses_and_mime() {
        let head = "From: =?UTF-8?B?T2xhIEtvd2Fsc2th?= <ola@x.pl>\r\nSubject: =?iso-8859-2?Q?Umowa_najmu_=B3=F3d=BC?=\r\nDate: Mon, 16 Nov 2026 10:00:00 +0100\r\nContent-Type: multipart/alternative; boundary=\"b1\"\r\n\r\n";
        let body = "--b1\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCzynsz 3 200 z=C5=82,\r\n do pi=C4=85tku.=\r\n\r\n--b1\r\nContent-Type: text/html\r\n\r\n<p>x</p>\r\n--b1--";
        let items: Vec<Item> = vec![
            (
                format!(
                    "* 3 FETCH (UID 812 BODY[HEADER.FIELDS (FROM SUBJECT DATE CONTENT-TYPE)] {{{}}}\r\n",
                    head.len()
                ),
                head.as_bytes().to_vec(),
            ),
            (
                format!(" BODY[TEXT]<0> {{{}}}\r\n", body.len()),
                body.as_bytes().to_vec(),
            ),
            (")\r\n".into(), vec![]),
        ];
        let m = &parse_fetch(&items)[0];
        assert_eq!(m.uid, 812);
        assert_eq!(m.from, "Ola Kowalska <ola@x.pl>");
        assert_eq!(m.subject, "Umowa najmu łódź");
        assert_eq!(m.text.trim(), "Czynsz 3 200 zł,\r\n do piątku.");
    }

    #[test]
    fn search_terms_are_stems() {
        assert_eq!(
            text_search("algebra Pogorzały"),
            r#"TEXT "algeb" TEXT "Pogorza""#
        );
        assert_eq!(text_search("umowa"), r#"TEXT "umow""#);
    }

    #[test]
    fn multipart_without_content_type() {
        let body = "--XYZ\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nHej, za\u{142}\u{105}cznik w &oacute;sme.\r\n--XYZ--";
        assert_eq!(
            body_text("", body.as_bytes()).trim(),
            "Hej, załącznik w ósme."
        );
    }

    #[test]
    fn base64_body_cut_short_still_reads() {
        let head =
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n";
        let b64 = base64::engine::general_purpose::STANDARD.encode("Spotkanie w piątek o 13.");
        assert!(
            body_text(head, &b64.as_bytes()[..b64.len() - 2]).starts_with("Spotkanie w piątek")
        );
    }
}
