//! Changing pictures with ImageMagick (`magick`, in the image): resize, convert,
//! compress, rotate, take out the location, and put several into one PDF. The
//! originals are never touched: results go to Pictures/Edited, and undo removes
//! them. A file name may have * and ? in it ("Pictures/Trip/*.jpg"), so the loop
//! over many files happens here, not in the model.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::files::{hidden, home_path, real_home, short};
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

const MAX_FILES: usize = 200;
const FORMATS: [&str; 4] = ["jpg", "png", "webp", "tif"];
const PICTURES: [&str; 11] = [
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "gif", "bmp", "heic", "heif", "avif",
];

/// The pictures among the files a path or pattern names.
fn pictures(raw: &str) -> Result<Vec<PathBuf>, ToolError> {
    let all = matching(raw)?;
    let found: Vec<PathBuf> = all
        .into_iter()
        .filter(|p| {
            p.extension()
                .is_some_and(|e| PICTURES.contains(&e.to_string_lossy().to_lowercase().as_str()))
        })
        .collect();
    if found.is_empty() {
        return Err(format!("no pictures in {}", raw.trim()).into());
    }
    Ok(found)
}

/// `*` and `?` in a file name.
fn wildcard(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (
        pattern.to_lowercase().chars().collect(),
        name.to_lowercase().chars().collect(),
    );
    // Classic two-pointer match with backtracking to the last *.
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

/// The files a path or a pattern names, sorted by name. Patterns work in the
/// last part only ("Pictures/Trip/*.jpg"); hidden files never match.
pub fn matching(raw: &str) -> Result<Vec<PathBuf>, ToolError> {
    let raw = raw.trim();
    let (dir, name) = match raw.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", raw),
    };
    // A folder means every file in it (the image tools keep the pictures).
    let (dir, name) = match home_path(raw, true) {
        Ok(p) if p.is_dir() => (raw.trim_end_matches('/'), "*"),
        Ok(p) if p.is_file() => return Ok(vec![p]),
        Ok(p) => return Err(format!("{} isn't a file", short(&p)).into()),
        Err(e) if !name.contains(['*', '?']) => return Err(e),
        Err(_) => (dir, name),
    };
    if dir.contains(['*', '?']) {
        return Err("* and ? work in the file name only, not in folder names".into());
    }
    let folder = if dir.is_empty() {
        real_home()
    } else {
        home_path(dir, true)?
    };
    let home = real_home();
    let mut found: Vec<PathBuf> = fs::read_dir(&folder)
        .map_err(|e| ToolError(format!("{}: {e}", short(&folder))))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && !hidden(p, &home))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| wildcard(name, &n.to_string_lossy()))
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return Err(format!("nothing in {} matches {name}", short(&folder)).into());
    }
    if found.len() > MAX_FILES {
        return Err(ToolError::cannot(format!(
            "{} files match; do at most {MAX_FILES} at a time",
            found.len()
        )));
    }
    Ok(found)
}

/// Pictures/Edited, made if missing.
fn edited_dir() -> Result<PathBuf, ToolError> {
    let home = real_home();
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)
        .and_then(|p| fs::canonicalize(p).ok())
        .filter(|p| p.starts_with(&home) && *p != home)
        .unwrap_or_else(|| home.join("Pictures"));
    let dir = pictures.join("Edited");
    fs::create_dir_all(&dir).map_err(|e| ToolError(format!("{}: {e}", short(&dir))))?;
    Ok(dir)
}

/// A name in `dir` that isn't taken: photo.jpg, photo-2.jpg…
fn free_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|i| dir.join(format!("{stem}-{i}.{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

fn size(p: &Path) -> String {
    let b = fs::metadata(p).map(|m| m.len()).unwrap_or(0) as f64;
    if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

fn dims(p: &Path) -> String {
    gdk_pixbuf::Pixbuf::file_info(p)
        .map(|(_, w, h)| format!("{w}×{h}"))
        .unwrap_or_else(|| "?".into())
}

/// Formats ImageMagick here can't read but GNOME's own loader (glycin, through
/// gdk-pixbuf) can: iPhone photos and the like.
const VIA_PIXBUF: [&str; 3] = ["heic", "heif", "avif"];

/// The file to hand to ImageMagick, and a temporary copy to remove after, if one was
/// made (a HEIC photo as PNG, turned the way the camera held it).
fn readable(f: &Path) -> Result<(PathBuf, Option<PathBuf>), ToolError> {
    let ext = f
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !VIA_PIXBUF.contains(&ext.as_str()) {
        return Ok((f.to_path_buf(), None));
    }
    let pb = gdk_pixbuf::Pixbuf::from_file(f)
        .map_err(|e| ToolError(format!("{} can't be read: {e}", short(f))))?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let tmp = std::env::temp_dir().join(format!(
        "panda-{}-{}.png",
        std::process::id(),
        f.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    pb.savev(&tmp, "png", &[])
        .map_err(|e| ToolError(format!("{} can't be converted: {e}", short(f))))?;
    Ok((tmp.clone(), Some(tmp)))
}

fn magick(args: &[String]) -> Result<(), ToolError> {
    let out = Command::new("magick")
        .args(args)
        .output()
        .map_err(|e| ToolError::cannot(format!("ImageMagick isn't available: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .next()
            .unwrap_or("ImageMagick failed")
            .to_string()
            .into())
    }
}

/// "50%" or a longest side in pixels ("1920").
fn resize_arg(raw: &str) -> Result<Option<String>, ToolError> {
    let r = raw.trim().trim_end_matches("px");
    if r.is_empty() {
        return Ok(None);
    }
    if let Some(p) = r.strip_suffix('%') {
        return match p.trim().parse::<f64>() {
            Ok(v) if v > 0.0 && v <= 400.0 => Ok(Some(format!("{v}%"))),
            _ => Err("resize: a percent from 1 to 400, like 50%".into()),
        };
    }
    match r.parse::<u32>() {
        Ok(v) if (16..=20000).contains(&v) => Ok(Some(format!("{v}x{v}>"))),
        _ => Err("resize: a percent (50%) or the longest side in pixels (1920)".into()),
    }
}

/// Remember the new files so undo removes them.
fn undo_removes(ctx: &Context, what: &str, made: Vec<PathBuf>) {
    ctx.can_undo(what, move || {
        let n = made.iter().filter(|p| fs::remove_file(p).is_ok()).count();
        Ok(format!(
            "removed {n} new file{}.",
            if n == 1 { "" } else { "s" }
        ))
    });
}

pub fn edit_image(ctx: &Context, args: &Args) -> ToolResult {
    let files = pictures(&arg_str(args, "files"))?;
    let resize = resize_arg(&arg_str(args, "resize"))?;
    let format = arg_str(args, "format")
        .trim()
        .to_lowercase()
        .replace("jpeg", "jpg");
    if !format.is_empty() && !FORMATS.contains(&format.as_str()) {
        return Err(format!("format: one of {}", FORMATS.join(", ")).into());
    }
    let quality = match arg_str(args, "quality").trim() {
        "" => None,
        q => match q.trim_end_matches('%').parse::<u32>() {
            Ok(v) if (1..=100).contains(&v) => Some(v),
            _ => return Err("quality: 1 to 100".into()),
        },
    };
    let rotate = match arg_str(args, "rotate").trim() {
        "" | "0" => None,
        r @ ("90" | "180" | "270" | "-90") => Some(r.to_string()),
        _ => return Err("rotate: 90, 180 or 270 (clockwise)".into()),
    };
    let strip = matches!(
        arg_str(args, "remove_location").trim(),
        "true" | "yes" | "1"
    );
    if resize.is_none() && format.is_empty() && quality.is_none() && rotate.is_none() && !strip {
        return Err(
            "say what to change: resize, format, quality, rotate or remove_location".into(),
        );
    }
    let dir = edited_dir()?;
    let mut made = vec![];
    let mut lines = vec![];
    for f in &files {
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = match f.extension().map(|e| e.to_string_lossy().to_lowercase()) {
            _ if !format.is_empty() => format.clone(),
            // Nothing here writes HEIC: those become JPG.
            Some(e) if !VIA_PIXBUF.contains(&e.as_str()) => e,
            _ => "jpg".into(),
        };
        let out = free_name(&dir, &stem, &ext);
        let (input, tmp) = match readable(f) {
            Ok(r) => r,
            Err(e) => {
                lines.push(format!("{}: failed ({})", short(f), e.0));
                continue;
            }
        };
        // [0]: the first frame of an animation or a multi-page file.
        let mut a = vec![format!("{}[0]", input.display()), "-auto-orient".into()];
        if let Some(r) = &resize {
            a.extend(["-resize".into(), r.clone()]);
        }
        if let Some(r) = &rotate {
            a.extend(["-rotate".into(), r.clone()]);
        }
        if strip {
            a.push("-strip".into());
        }
        if let Some(q) = quality {
            a.extend(["-quality".into(), q.to_string()]);
        }
        a.push(out.display().to_string());
        let done = magick(&a);
        if let Some(t) = tmp {
            let _ = fs::remove_file(t);
        }
        match done {
            Ok(()) => {
                lines.push(format!(
                    "{}: {} {} → {} {}",
                    short(&out),
                    size(f),
                    dims(f),
                    size(&out),
                    dims(&out)
                ));
                made.push(out);
            }
            Err(e) => lines.push(format!("{}: failed ({})", short(f), e.0)),
        }
    }
    if made.is_empty() {
        return Err(format!("no picture could be changed:\n{}", lines.join("\n")).into());
    }
    let n = made.len();
    undo_removes(
        ctx,
        &format!("Edit {n} picture{}", if n == 1 { "" } else { "s" }),
        made,
    );
    Ok(format!(
        "Saved {n} of {} in {} (the originals are unchanged):\n{}",
        files.len(),
        short(&dir),
        lines.join("\n")
    ))
}

pub fn images_to_pdf(ctx: &Context, args: &Args) -> ToolResult {
    let files = pictures(&arg_str(args, "files"))?;
    let name = arg_str(args, "name");
    let name = name.trim().trim_end_matches(".pdf");
    let stem: String = if name.is_empty() { "pictures" } else { name }
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':'))
        .collect();
    let out = free_name(&edited_dir()?, &stem, "pdf");
    let inputs = files
        .iter()
        .map(|f| readable(f))
        .collect::<Result<Vec<_>, _>>()?;
    let mut a: Vec<String> = inputs
        .iter()
        .map(|(p, _)| format!("{}[0]", p.display()))
        .collect();
    a.extend(["-auto-orient".into(), out.display().to_string()]);
    let done = magick(&a);
    for t in inputs.into_iter().filter_map(|(_, t)| t) {
        let _ = fs::remove_file(t);
    }
    done?;
    let line = format!(
        "Made {} from {} pictures ({})",
        short(&out),
        files.len(),
        size(&out)
    );
    undo_removes(ctx, &format!("Make {}", short(&out)), vec![out]);
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard("*.jpg", "IMG_1.JPG"));
        assert!(wildcard("img_?.jpg", "img_1.jpg"));
        assert!(!wildcard("img_?.jpg", "img_12.jpg"));
        assert!(wildcard("*scan*", "2026 scan 3.pdf"));
        assert!(!wildcard("*.png", "a.jpg"));
        assert!(wildcard("*", "anything"));
    }

    /// End to end with real files, when ImageMagick is installed.
    #[test]
    fn edit_pictures_by_pattern_and_undo() {
        if Command::new("magick").arg("-version").output().is_err() {
            return;
        }
        let _h = crate::store::testenv::temp_home();
        let trip = real_home().join("Trip");
        fs::create_dir_all(&trip).unwrap();
        for n in ["a.png", "b.png", "notes.txt"] {
            let pb =
                gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 400, 200).unwrap();
            pb.fill(0x3366ffff);
            if n.ends_with(".png") {
                pb.savev(trip.join(n), "png", &[]).unwrap();
            } else {
                fs::write(trip.join(n), "x").unwrap();
            }
        }
        let ctx = Context::default();
        let args: Args = serde_json::from_value(
            serde_json::json!({"files": "Trip/*.png", "resize": "50%", "format": "jpg"}),
        )
        .unwrap();
        let out = edit_image(&ctx, &args).unwrap();
        assert!(out.starts_with("Saved 2 of 2"), "{out}");
        let made = real_home().join("Pictures/Edited/a.jpg");
        assert_eq!(
            gdk_pixbuf::Pixbuf::file_info(&made).map(|(_, w, h)| (w, h)),
            Some((200, 100))
        );
        assert!(trip.join("a.png").exists(), "the original stays");
        let pdf: Args =
            serde_json::from_value(serde_json::json!({"files": "Trip/*.png", "name": "trip"}))
                .unwrap();
        assert!(
            images_to_pdf(&ctx, &pdf)
                .unwrap()
                .contains("from 2 pictures")
        );
        assert!(real_home().join("Pictures/Edited/trip.pdf").exists());
        ctx.undo_task().unwrap();
        assert!(!made.exists() && !real_home().join("Pictures/Edited/trip.pdf").exists());
        let whole: Args =
            serde_json::from_value(serde_json::json!({"files": "Trip", "rotate": 90})).unwrap();
        assert!(
            edit_image(&ctx, &whole)
                .unwrap()
                .starts_with("Saved 2 of 2"),
            "a folder: its pictures only"
        );
        assert!(matching("Trip/*.gif").is_err());
        assert!(matching("*/a.png").is_err());
    }

    /// An iPhone-style HEIC photo (libheif's example), when it was downloaded for the test.
    #[test]
    fn heic_goes_through_gnomes_loader() {
        let sample = std::env::var("PANDA_HEIC_SAMPLE").unwrap_or_default();
        if sample.is_empty() || Command::new("magick").arg("-version").output().is_err() {
            return;
        }
        let _h = crate::store::testenv::temp_home();
        let dir = real_home().join("Photos");
        fs::create_dir_all(&dir).unwrap();
        fs::copy(&sample, dir.join("IMG_0001.HEIC")).unwrap();
        let ctx = Context::default();
        let args: Args =
            serde_json::from_value(serde_json::json!({"files": "Photos", "resize": "50%"}))
                .unwrap();
        let out = edit_image(&ctx, &args).unwrap();
        assert!(out.starts_with("Saved 1 of 1"), "{out}");
        let made = real_home().join("Pictures/Edited/IMG_0001.jpg");
        assert_eq!(
            gdk_pixbuf::Pixbuf::file_info(&made).map(|(_, w, h)| (w, h)),
            Some((640, 427))
        );
        let pdf: Args = serde_json::from_value(serde_json::json!({"files": "Photos"})).unwrap();
        assert!(
            images_to_pdf(&ctx, &pdf)
                .unwrap()
                .contains("from 1 pictures")
        );
    }

    #[test]
    fn resize_values() {
        assert_eq!(resize_arg("50%").unwrap().unwrap(), "50%");
        assert_eq!(resize_arg("1920px").unwrap().unwrap(), "1920x1920>");
        assert!(resize_arg("-resize").is_err());
        assert!(resize_arg("0%").is_err());
        assert_eq!(resize_arg("").unwrap(), None);
    }
}
