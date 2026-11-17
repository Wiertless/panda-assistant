//! Playing music from the user's library: "play Bohemian Rhapsody in
//! Gapless", "puść album Random Access Memories". Tracks come from GNOME's
//! file index (title, artist, album), or from the Music folder when the
//! index has none; they open in the app asked for, or the default player.

use std::path::{Path, PathBuf};

use gio::prelude::*;

use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const MUSIC_QUERY: &str = "SELECT ?url ?title ?artist ?album ?track WHERE { \
?song a nmm:MusicPiece ; nie:isStoredAs ?f . ?f nie:url ?url . \
OPTIONAL { ?song nie:title ?title } \
OPTIONAL { ?song nmm:artist ?a . ?a nmm:artistName ?artist } \
OPTIONAL { ?song nmm:musicAlbum ?al . ?al nie:title ?album } \
OPTIONAL { ?song nmm:trackNumber ?track } } LIMIT 20000";
const AUDIO: [&str; 9] = [
    "mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "wma", "alac",
];
const MAX_QUEUE: usize = 200;
/// Words of a request that don't name the music.
const FILLER: [&str; 22] = [
    "play",
    "song",
    "songs",
    "track",
    "music",
    "the",
    "by",
    "some",
    "album",
    "artist",
    "from",
    "me",
    "please",
    "puść",
    "pusc",
    "zagraj",
    "włącz",
    "wlacz",
    "piosenkę",
    "piosenke",
    "utwór",
    "album",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub number: u32,
}

impl Track {
    fn label(&self) -> String {
        match (self.title.is_empty(), self.artist.is_empty()) {
            (false, false) => format!("{} by {}", self.title, self.artist),
            (false, true) => self.title.clone(),
            _ => gio::File::for_uri(&self.uri)
                .basename()
                .map(|b| b.to_string_lossy().to_string())
                .unwrap_or_default(),
        }
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect()
}

fn words(query: &str) -> Vec<String> {
    norm(query)
        .split_whitespace()
        .filter(|w| !FILLER.contains(w))
        .map(String::from)
        .collect()
}

fn indexed() -> Vec<Track> {
    let Ok(s) = super::localsearch::sparql() else {
        return vec![];
    };
    let Ok(conn) = s.connect() else { return vec![] };
    s.query(&conn, MUSIC_QUERY, &[])
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| {
            let uri = r.first()?.clone();
            Some(Track {
                uri,
                title: r.get(1).cloned().unwrap_or_default(),
                artist: r.get(2).cloned().unwrap_or_default(),
                album: r.get(3).cloned().unwrap_or_default(),
                number: r.get(4).and_then(|n| n.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

/// Audio files under the Music folder (when the index has none).
fn scanned(dir: &Path) -> Vec<Track> {
    let mut out = vec![];
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p: PathBuf = e.path();
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if p.is_dir() && depth < 6 {
                stack.push((p, depth + 1));
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| AUDIO.contains(&x.to_lowercase().as_str()))
            {
                // Folder layout as the tags: Artist/Album/NN Title.ext
                let title = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let album = p
                    .parent()
                    .and_then(|a| a.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let artist = p
                    .parent()
                    .and_then(|a| a.parent())
                    .filter(|a| *a != dir)
                    .and_then(|a| a.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let number = title
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                out.push(Track {
                    uri: gio::File::for_path(&p).uri().to_string(),
                    title,
                    artist,
                    album,
                    number,
                });
            }
            if out.len() > 20000 {
                return out;
            }
        }
    }
    out
}

/// The tracks a request means, in play order: the best matches; when they
/// share an album or artist that the request names, all of it, in order.
pub fn choose(tracks: &[Track], query: &str) -> Vec<Track> {
    let want = words(query);
    if want.is_empty() {
        return vec![];
    }
    let score = |t: &Track| -> usize {
        let hay = format!(
            " {} {} {} {} ",
            norm(&t.title),
            norm(&t.artist),
            norm(&t.album),
            norm(&t.uri)
        );
        want.iter()
            .filter(|w| hay.contains(&format!(" {w}")))
            .count()
    };
    let best = tracks.iter().map(score).max().unwrap_or(0);
    if best == 0 || best * 2 < want.len() {
        return vec![];
    }
    let mut hits: Vec<Track> = tracks
        .iter()
        .filter(|t| score(t) == best)
        .cloned()
        .collect();
    // A song by name: the one whose title matches best goes first.
    let title_hits = |t: &Track| {
        want.iter()
            .filter(|w| {
                norm(&t.title)
                    .split_whitespace()
                    .any(|x| x.starts_with(w.as_str()))
            })
            .count()
    };
    hits.sort_by(|a, b| {
        title_hits(b)
            .cmp(&title_hits(a))
            .then(a.album.cmp(&b.album))
            .then(a.number.cmp(&b.number))
            .then(a.uri.cmp(&b.uri))
    });
    let names_title = hits.first().is_some_and(|t| {
        title_hits(t) * 2 >= want.len() && !want.iter().all(|w| norm(&t.album).contains(w.as_str()))
    });
    if names_title {
        hits.truncate(1);
    } else {
        hits.sort_by(|a, b| {
            a.artist
                .cmp(&b.artist)
                .then(a.album.cmp(&b.album))
                .then(a.number.cmp(&b.number))
                .then(a.uri.cmp(&b.uri))
        });
        hits.truncate(MAX_QUEUE);
    }
    hits
}

pub fn play_music(_: &Context, args: &Args) -> ToolResult {
    let query = arg_str(args, "query");
    let mut tracks = indexed();
    if tracks.is_empty()
        && let Some(dir) = glib::user_special_dir(glib::UserDirectory::Music)
    {
        tracks = scanned(&dir);
    }
    if tracks.is_empty() {
        return Err(ToolError::cannot(
            "no music found in your library (the Music folder is empty or not indexed)",
        ));
    }
    let pick = choose(&tracks, &query);
    if pick.is_empty() {
        return Err(ToolError::cannot(format!(
            "no song, album or artist matching “{}” in your music library",
            query.trim()
        )));
    }
    let app_name = arg_str(args, "app");
    let app = if app_name.trim().is_empty() {
        super::apps::default_app("music")
            .ok_or_else(|| ToolError::cannot("no music player app is installed"))?
    } else {
        super::apps::find_app(&app_name)?
    };
    let uris: Vec<&str> = pick.iter().map(|t| t.uri.as_str()).collect();
    app.launch_uris(&uris, None::<&gio::AppLaunchContext>)
        .map_err(|e| ToolError(format!("{} didn't open it: {e}", app.display_name())))?;
    Ok(if pick.len() == 1 {
        format!("Playing {} in {}.", pick[0].label(), app.display_name())
    } else {
        let what = if !pick[0].album.is_empty() && pick.iter().all(|t| t.album == pick[0].album) {
            pick[0].album.clone()
        } else {
            pick[0].artist.clone()
        };
        format!(
            "Playing {} songs{} in {}.",
            pick.len(),
            if what.is_empty() {
                String::new()
            } else {
                format!(" ({what})")
            },
            app.display_name()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, album: &str, n: u32) -> Track {
        Track {
            uri: format!("file:///m/{artist}/{album}/{n:02} {title}.flac"),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            number: n,
        }
    }

    #[test]
    fn songs_albums_artists() {
        let lib = vec![
            t("Bohemian Rhapsody", "Queen", "A Night at the Opera", 11),
            t("Love of My Life", "Queen", "A Night at the Opera", 9),
            t("Get Lucky", "Daft Punk", "Random Access Memories", 8),
            t(
                "Giorgio by Moroder",
                "Daft Punk",
                "Random Access Memories",
                3,
            ),
            t(
                "Lose Yourself to Dance",
                "Daft Punk",
                "Random Access Memories",
                6,
            ),
        ];
        let song = choose(&lib, "play Bohemian Rhapsody");
        assert_eq!(
            song.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Bohemian Rhapsody"]
        );
        let album = choose(&lib, "puść album Random Access Memories");
        assert_eq!(
            album.iter().map(|t| t.number).collect::<Vec<_>>(),
            [3, 6, 8],
            "the whole album, in order"
        );
        assert_eq!(choose(&lib, "play some Queen").len(), 2);
        assert!(
            choose(&lib, "play Taylor Swift").is_empty(),
            "nothing like it: no guess"
        );
        assert_eq!(choose(&lib, "get lucky by daft punk")[0].title, "Get Lucky");
    }

    #[test]
    fn the_index_query_finds_tagged_songs() {
        use super::super::localsearch::{Conn, sparql};
        let Ok(s) = sparql() else { return }; // no TinySPARQL here
        let mut err = std::ptr::null_mut();
        // SAFETY: as in localsearch's own test: in-memory store, Nepomuk ontology.
        let conn = Conn(unsafe {
            (s.new_local)(
                1 << 1 | 1 << 2,
                std::ptr::null_mut(),
                (s.nepomuk)(),
                std::ptr::null_mut(),
                &mut err,
            )
        });
        if conn.0.is_null() {
            return;
        }
        let q = std::ffi::CString::new(r#"INSERT DATA {
            <file:///m/q.flac> a nfo:FileDataObject ; nie:url "file:///m/q.flac" .
            <urn:artist:queen> a nmm:Artist ; nmm:artistName "Queen" .
            <urn:album:opera> a nmm:MusicAlbum ; nie:title "A Night at the Opera" .
            <urn:song:1> a nmm:MusicPiece ; nie:isStoredAs <file:///m/q.flac> ; nie:title "Bohemian Rhapsody" ;
                nmm:artist <urn:artist:queen> ; nmm:musicAlbum <urn:album:opera> ; nmm:trackNumber 11 . }"#).unwrap();
        let mut err = std::ptr::null_mut();
        // SAFETY: valid connection and C string.
        unsafe { (s.update)(conn.0, q.as_ptr(), std::ptr::null_mut(), &mut err) };
        if !err.is_null() {
            eprintln!("skipping: this TinySPARQL can't build the test index");
            return;
        }
        let rows = s.query(&conn, MUSIC_QUERY, &[]).unwrap();
        assert_eq!(
            rows,
            [vec![
                "file:///m/q.flac",
                "Bohemian Rhapsody",
                "Queen",
                "A Night at the Opera",
                "11"
            ]]
        );
    }

    #[test]
    fn music_folder_layout() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Queen/A Night at the Opera");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("11 Bohemian Rhapsody.flac"), "x").unwrap();
        std::fs::write(album.join("cover.jpg"), "x").unwrap();
        let found = scanned(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(
            (
                found[0].artist.as_str(),
                found[0].album.as_str(),
                found[0].number
            ),
            ("Queen", "A Night at the Opera", 11)
        );
    }
}
