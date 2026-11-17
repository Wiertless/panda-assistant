//! Files by what's in them, from GNOME's LocalSearch index (the one Files
//! and the overview search use), including text it extracted from PDFs and
//! office documents. Queries go through TinySPARQL's C library, loaded at
//! run time: it's called libtinysparql on Fedora 41+ and libtracker-sparql on
//! older systems, and loading it late means no build-time dependency either.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gio::prelude::*;

use super::files::{hidden, home_path, short};
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const LIBRARIES: [&str; 2] = ["libtinysparql-3.0.so.0", "libtracker-sparql-3.0.so.0"];
const INDEXER_BUSES: [&str; 2] = ["org.freedesktop.LocalSearch3", "org.freedesktop.Tracker3.Miner.Files"];
const MAX_HITS: usize = 10;
const MAX_TEXT: usize = 3000;
const FTS_SKIP: [&str; 12] = ["and", "or", "not", "near", "the", "a", "an", "of", "in", "file", "files", "about"];

const CONTENT_QUERY: &str = r#"
SELECT DISTINCT ?url ?title fts:snippet(?ie, "", "", "…", 14) WHERE {
  ?ie fts:match ~q ;
      nie:isStoredAs ?file .
  ?file nie:url ?url .
  OPTIONAL { ?ie nie:title ?title }
} ORDER BY DESC(fts:rank(?ie)) LIMIT 40
"#;

const TEXT_QUERY: &str = r#"
SELECT ?text WHERE {
  ?ie nie:isStoredAs ?file ;
      nie:plainTextContent ?text .
  ?file nie:url ~url .
} LIMIT 1
"#;

type GError = glib::ffi::GError;

/// The few TinySPARQL functions used here.
pub struct Sparql {
    _lib: libloading::Library,
    bus_new: unsafe extern "C" fn(*const c_char, *const c_char, *mut c_void, *mut *mut GError) -> *mut c_void,
    query_statement: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, *mut *mut GError) -> *mut c_void,
    bind_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char),
    execute: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut GError) -> *mut c_void,
    cursor_next: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut GError) -> c_int,
    get_string: unsafe extern "C" fn(*mut c_void, c_int, *mut c_long) -> *const c_char,
    n_columns: unsafe extern "C" fn(*mut c_void) -> c_int,
    cursor_close: unsafe extern "C" fn(*mut c_void),
    /// For tests: an in-memory database with GNOME's (Nepomuk) schema.
    pub new_local: unsafe extern "C" fn(c_int, *mut c_void, *mut c_void, *mut c_void, *mut *mut GError) -> *mut c_void,
    pub nepomuk: unsafe extern "C" fn() -> *mut c_void,
    pub update: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, *mut *mut GError),
}

fn take_error(err: *mut GError) -> String {
    if err.is_null() {
        return "unknown error".into();
    }
    // SAFETY: a GError set by the library; we own it and free it here.
    unsafe {
        let msg = CStr::from_ptr((*err).message).to_string_lossy().into_owned();
        glib::ffi::g_error_free(err);
        msg
    }
}

pub fn sparql() -> Result<&'static Sparql, ToolError> {
    static LIB: OnceLock<Option<Sparql>> = OnceLock::new();
    LIB.get_or_init(|| {
        // SAFETY: loading a system library and looking up functions by their documented C signatures.
        unsafe {
            let lib = LIBRARIES.iter().find_map(|n| libloading::Library::new(n).ok())?;
            macro_rules! sym { ($n:expr) => { *lib.get($n).ok()? } }
            Some(Sparql {
                bus_new: sym!(b"tracker_sparql_connection_bus_new\0"),
                query_statement: sym!(b"tracker_sparql_connection_query_statement\0"),
                bind_string: sym!(b"tracker_sparql_statement_bind_string\0"),
                execute: sym!(b"tracker_sparql_statement_execute\0"),
                cursor_next: sym!(b"tracker_sparql_cursor_next\0"),
                get_string: sym!(b"tracker_sparql_cursor_get_string\0"),
                n_columns: sym!(b"tracker_sparql_cursor_get_n_columns\0"),
                cursor_close: sym!(b"tracker_sparql_cursor_close\0"),
                new_local: sym!(b"tracker_sparql_connection_new\0"),
                nepomuk: sym!(b"tracker_sparql_get_ontology_nepomuk\0"),
                update: sym!(b"tracker_sparql_connection_update\0"),
                _lib: lib,
            })
        }
    }).as_ref().ok_or_else(|| ToolError::cannot("GNOME's file index (LocalSearch) isn't installed"))
}

/// A connection to the indexer (or, in tests, any connection).
pub struct Conn(pub *mut c_void);

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: we own this reference.
        unsafe { glib::gobject_ffi::g_object_unref(self.0 as *mut _) }
    }
}

impl Sparql {
    pub fn query(&self, conn: &Conn, sparql: &str, binds: &[(&str, &str)]) -> Result<Vec<Vec<String>>, ToolError> {
        let q = CString::new(sparql).unwrap();
        let mut err = std::ptr::null_mut();
        // SAFETY: valid connection and C strings; every object we get is unreffed below.
        unsafe {
            let stmt = (self.query_statement)(conn.0, q.as_ptr(), std::ptr::null_mut(), &mut err);
            if stmt.is_null() {
                return Err(format!("the file index couldn't search: {}", take_error(err)).into());
            }
            for (k, v) in binds {
                let (k, v) = (CString::new(*k).unwrap(), CString::new(*v).unwrap_or_default());
                (self.bind_string)(stmt, k.as_ptr(), v.as_ptr());
            }
            let cursor = (self.execute)(stmt, std::ptr::null_mut(), &mut err);
            glib::gobject_ffi::g_object_unref(stmt as *mut _);
            if cursor.is_null() {
                return Err(format!("the file index couldn't search: {}", take_error(err)).into());
            }
            let cols = (self.n_columns)(cursor);
            let mut rows = vec![];
            while (self.cursor_next)(cursor, std::ptr::null_mut(), &mut err) != 0 {
                rows.push((0..cols).map(|i| {
                    let p = (self.get_string)(cursor, i, std::ptr::null_mut());
                    if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
                }).collect());
            }
            (self.cursor_close)(cursor);
            glib::gobject_ffi::g_object_unref(cursor as *mut _);
            if !err.is_null() {
                return Err(format!("the file index couldn't search: {}", take_error(err)).into());
            }
            Ok(rows)
        }
    }

    fn connect(&self) -> Result<Conn, ToolError> {
        for bus in INDEXER_BUSES {
            let name = CString::new(bus).unwrap();
            let mut err = std::ptr::null_mut();
            // SAFETY: a bus name and NULL path/connection (defaults), as documented.
            let conn = unsafe { (self.bus_new)(name.as_ptr(), std::ptr::null(), std::ptr::null_mut(), &mut err) };
            if conn.is_null() {
                take_error(err);
                continue;
            }
            let conn = Conn(conn);
            if self.query(&conn, "ASK {}", &[]).is_ok() {
                return Ok(conn);
            }
        }
        Err(ToolError::cannot("GNOME's file index (LocalSearch) isn't running"))
    }
}

/// User words -> an FTS5 match: every word, as a prefix; no operators.
pub fn fts_terms(text: &str) -> Result<String, ToolError> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.chars().count() > 1 && !FTS_SKIP.contains(w)).take(8).collect();
    if words.is_empty() {
        return Err("say what to look for".into());
    }
    Ok(words.iter().map(|w| format!("{w}*")).collect::<Vec<_>>().join(" "))
}

fn url_to_path(url: &str) -> Option<PathBuf> {
    gio::File::for_uri(url).path().filter(|_| url.starts_with("file://"))
}

/// Search with an open connection (split out so tests can use a local database).
pub fn search_with(s: &Sparql, conn: &Conn, query: &str) -> ToolResult {
    let home = std::fs::canonicalize(crate::paths::home()).unwrap_or_else(|_| crate::paths::home());
    let mut lines = vec![];
    for row in s.query(conn, CONTENT_QUERY, &[("q", &fts_terms(query)?)])? {
        let (url, title, snippet) = (&row[0], &row[1], &row[2]);
        let Some(path) = url_to_path(url) else { continue };
        if !path.starts_with(&home) || hidden(&path, &home) {
            continue; // stay inside the home folder, away from hidden files
        }
        let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let mut line = short(&path);
        if !title.is_empty() && *title != name {
            line.push_str(&format!(" — {title}"));
        }
        if !snippet.is_empty() {
            line.push_str(&format!("\n  “{}”", snippet.chars().take(200).collect::<String>()));
        }
        lines.push(line);
        if lines.len() >= MAX_HITS {
            break;
        }
    }
    Ok(if lines.is_empty() { format!("No files mention “{query}” (only indexed folders are searched).") } else { lines.join("\n") })
}

pub fn indexed_text_with(s: &Sparql, conn: &Conn, path: &Path) -> String {
    let uri = gio::File::for_path(path).uri();
    s.query(conn, TEXT_QUERY, &[("url", uri.as_str())]).ok()
        .and_then(|rows| rows.into_iter().next()).and_then(|r| r.into_iter().next()).unwrap_or_default()
}

pub fn search_file_contents(_: &Context, args: &Args) -> ToolResult {
    let s = sparql()?;
    search_with(s, &s.connect()?, &arg_str(args, "query"))
}

/// A document's text: plain text read directly, others from GNOME's index,
/// PDFs from pdftotext (up to `pages`) when not indexed yet.
pub fn document_text(path: &Path, pages: u32) -> Result<String, ToolError> {
    if !path.is_file() {
        return Err(format!("{} isn't a file", short(path)).into());
    }
    let plain = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["txt", "md", "markdown", "csv", "log", "org", "rst", "json", "toml", "yaml", "yml", "ini", "html", "htm"]
            .contains(&e.to_lowercase().as_str())
    });
    if plain {
        let bytes = std::fs::read(path).map_err(|e| ToolError(e.to_string()))?;
        return Ok(String::from_utf8_lossy(&bytes[..bytes.len().min(2_000_000)]).to_string());
    }
    let mut text = sparql().ok().and_then(|s| Some(indexed_text_with(s, &s.connect().ok()?, path))).unwrap_or_default();
    if text.trim().is_empty() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        && let Ok(out) = super::run::output(&["pdftotext", "-layout", "-l", &pages.to_string(), &path.to_string_lossy(), "-"],
                                            std::time::Duration::from_secs(40), None) {
            text = out.stdout;
        }
    if text.trim().is_empty() {
        return Err(format!("no text is available for {} (it may not be indexed yet)", short(path)).into());
    }
    Ok(text)
}

pub fn read_document(_: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    let text = document_text(&path, 20)?;
    let mut lines = vec![];
    let mut blank = 0;
    for l in text.trim().lines() {
        blank = if l.trim().is_empty() { blank + 1 } else { 0 };
        if blank < 2 {
            lines.push(l);
        }
    }
    let text = lines.join("\n");
    let body = if text.chars().count() > MAX_TEXT { format!("{}\n[…]", text.chars().take(MAX_TEXT).collect::<String>()) } else { text };
    Ok(format!("{}:\n{body}", short(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testenv::temp_home;

    #[test]
    fn fts_terms_are_plain_prefixes() {
        assert_eq!(fts_terms("the file about lease \"Marek\" OR* x").unwrap(), "lease* marek*");
        assert!(fts_terms("?!").is_err());
    }

    #[test]
    fn queries_against_an_in_memory_index() {
        let Ok(s) = sparql() else { return }; // no TinySPARQL here
        let _h = temp_home();
        let home = std::fs::canonicalize(crate::paths::home()).unwrap();
        let mut err = std::ptr::null_mut();
        // FTS stemmer + unaccent flags; in-memory store; Nepomuk ontology.
        let conn = Conn(unsafe { (s.new_local)(1 << 1 | 1 << 2, std::ptr::null_mut(), (s.nepomuk)(), std::ptr::null_mut(), &mut err) });
        assert!(!conn.0.is_null(), "{}", take_error(err));
        for (i, (rel, title, text)) in [("Dokumenty/lease.pdf", "Lease agreement", "Lease agreement with Marek Nowak. Rent is 2000 zloty."),
                                        (".ssh/notes.txt", "", "lease secret")].iter().enumerate() {
            let url = gio::File::for_path(home.join(rel)).uri();
            let q = CString::new(format!(r#"INSERT DATA {{ <{url}> a nfo:FileDataObject ; nie:url "{url}" .
                <urn:ie:{i}> a nfo:Document ; nie:isStoredAs <{url}> ; nie:title "{title}" ; nie:plainTextContent "{text}" . }}"#)).unwrap();
            let mut err = std::ptr::null_mut();
            unsafe { (s.update)(conn.0, q.as_ptr(), std::ptr::null_mut(), &mut err) };
            if !err.is_null() {
                // Some older libtracker-sparql builds can't write to an in-memory
                // store with newer SQLite ("ambiguous column name: ROWID"). That's
                // the test fixture failing, not the queries under test.
                eprintln!("skipping: this TinySPARQL can't build the test index: {}", take_error(err));
                return;
            }
        }
        let out = search_with(s, &conn, "rent marek").unwrap();
        assert!(out.starts_with("~/Dokumenty/lease.pdf — Lease agreement\n  “Lease agreement with Marek"), "{out}");
        assert!(!search_with(s, &conn, "leas").unwrap().contains(".ssh"));
        assert!(search_with(s, &conn, "zzz").unwrap().starts_with("No files mention"));
        assert!(indexed_text_with(s, &conn, &home.join("Dokumenty/lease.pdf")).contains("Rent is 2000"));
    }
}
