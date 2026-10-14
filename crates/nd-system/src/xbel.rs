//! The freedesktop recent files list, `$XDG_DATA_HOME/recently-used.xbel`, which GTK, KDE and
//! file pickers read. Edits are textual and touch only this app's entries: an existing
//! bookmark keeps every other app's registration, and clearing removes the app from each
//! bookmark (the bookmark goes once no app is left). Written atomically.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{AppIdentity, Error, Result};

const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info"
>
</xbel>
"#;

pub fn default_path() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    data.join("recently-used.xbel")
}

pub(crate) fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => uri.push(*byte as char),
            b => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// `2026-10-14T09:30:00.000000Z`.
fn timestamp(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn application(app: &AppIdentity, now: &str, count: u32) -> String {
    let exec = escape(&format!("'{} %u'", app.name));
    format!(
        "          <bookmark:application name=\"{}\" exec=\"{exec}\" modified=\"{now}\" count=\"{count}\"/>",
        escape(&app.display_name)
    )
}

/// The byte range of each `<bookmark ...>...</bookmark>` block.
fn blocks(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find("<bookmark ").map(|i| i + from) {
        let Some(end) = text[start..].find("</bookmark>").map(|i| start + i + "</bookmark>".len()) else { break };
        out.push((start, end));
        from = end;
    }
    out
}

/// The `<bookmark:application .../>` line naming `app` inside `block`, as a range in `block`.
fn our_application(block: &str, app: &AppIdentity) -> Option<(usize, usize)> {
    let needle = format!("<bookmark:application name=\"{}\"", escape(&app.display_name));
    let start = block.find(&needle)?;
    let end = start + block[start..].find("/>")? + 2;
    let line_start = block[..start].rfind('\n').map_or(start, |i| i + 1);
    let line_end = if block[end..].starts_with('\n') { end + 1 } else { end };
    Some((line_start, line_end))
}

fn set_attr(tag: &str, name: &str, value: &str) -> String {
    let key = format!(" {name}=\"");
    match tag.find(&key) {
        Some(i) => {
            let v = i + key.len();
            let close = tag[v..].find('"').map_or(tag.len(), |j| v + j);
            format!("{}{value}{}", &tag[..v], &tag[close..])
        }
        None => match tag.rfind('>') {
            Some(gt) => format!("{} {name}=\"{value}\"{}", &tag[..gt], &tag[gt..]),
            None => tag.to_owned(),
        },
    }
}

fn read(file: &Path) -> Result<String> {
    match std::fs::read_to_string(file) {
        Ok(text) if text.contains("</xbel>") => Ok(text),
        Ok(_) => Err(Error::Platform(format!("{}: not an XBEL file", file.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(EMPTY.to_owned()),
        Err(e) => Err(Error::platform(e)),
    }
}

fn write(file: &Path, text: &str) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(Error::platform)?;
    }
    let tmp = file.with_extension(format!("xbel.nd-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(Error::platform)?;
    std::fs::rename(&tmp, file).map_err(Error::platform)
}

pub fn add(file: &Path, app: &AppIdentity, path: &Path) -> Result<()> {
    add_at(file, app, path, SystemTime::now())
}

fn add_at(file: &Path, app: &AppIdentity, path: &Path, at: SystemTime) -> Result<()> {
    let text = read(file)?;
    let now = timestamp(at);
    let href = escape(&file_uri(path));
    let opening = format!("<bookmark href=\"{href}\"");
    let existing = blocks(&text).into_iter().find(|&(s, e)| text[s..e].starts_with(&opening));
    let updated = match existing {
        Some((s, e)) => {
            let block = &text[s..e];
            let tag_end = block.find('>').map_or(block.len(), |i| i + 1);
            let mut tag = block[..tag_end].to_owned();
            for attr in ["modified", "visited"] {
                tag = set_attr(&tag, attr, &now);
            }
            let mut body = block[tag_end..].to_owned();
            match our_application(&body, app) {
                Some((ls, le)) => {
                    let line = &body[ls..le];
                    let count = line
                        .split("count=\"")
                        .nth(1)
                        .and_then(|c| c.split('"').next())
                        .and_then(|c| c.parse::<u32>().ok())
                        .unwrap_or(0);
                    let newline = if line.ends_with('\n') { "\n" } else { "" };
                    body.replace_range(ls..le, &format!("{}{newline}", application(app, &now, count + 1)));
                }
                None => match body.find("</bookmark:applications>") {
                    Some(i) => {
                        let line_start = body[..i].rfind('\n').map_or(i, |j| j + 1);
                        body.insert_str(line_start, &format!("{}\n", application(app, &now, 1)));
                    }
                    None => return Err(Error::Platform("recent bookmark without an applications list".into())),
                },
            }
            format!("{}{tag}{body}{}", &text[..s], &text[e..])
        }
        None => {
            let mime = "application/octet-stream";
            let block = format!(
                "  <bookmark href=\"{href}\" added=\"{now}\" modified=\"{now}\" visited=\"{now}\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"{mime}\"/>\n        <bookmark:applications>\n{}\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n",
                application(app, &now, 1)
            );
            let at = text.rfind("</xbel>").expect("checked by read");
            format!("{}{block}{}", &text[..at], &text[at..])
        }
    };
    write(file, &updated)
}

pub fn clear(file: &Path, app: &AppIdentity) -> Result<()> {
    let text = read(file)?;
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for (s, e) in blocks(&text) {
        let block = &text[s..e];
        let Some((ls, le)) = our_application(block, app) else { continue };
        let others = block.matches("<bookmark:application ").count() > 1;
        if others {
            out.push_str(&text[from..s]);
            out.push_str(&block[..ls]);
            out.push_str(&block[le..]);
            from = e;
        } else {
            let line_start = text[..s].rfind('\n').map_or(s, |i| i + 1);
            out.push_str(&text[from..line_start]);
            from = if text[e..].starts_with('\n') { e + 1 } else { e };
        }
    }
    out.push_str(&text[from..]);
    write(file, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> AppIdentity {
        AppIdentity { id: format!("dev.nd.{name}"), name: name.into(), display_name: name.into() }
    }

    #[test]
    fn file_uris_escape_what_a_uri_cannot_hold() {
        assert_eq!(file_uri("/tmp/a b/ü#.txt".as_ref()), "file:///tmp/a%20b/%C3%BC%23.txt");
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000000Z");
        assert_eq!(
            timestamp(UNIX_EPOCH + std::time::Duration::from_secs(1_791_970_200)),
            "2026-10-14T09:30:00.000000Z"
        );
    }

    #[test]
    fn adds_bumps_and_clears_only_this_app() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recently-used.xbel");
        let (notes, other) = (app("notes"), app("gedit"));
        let t = |s| UNIX_EPOCH + std::time::Duration::from_secs(s);

        add_at(&file, &notes, Path::new("/tmp/a b.txt"), t(100)).unwrap();
        add_at(&file, &other, Path::new("/tmp/a b.txt"), t(200)).unwrap();
        add_at(&file, &notes, Path::new("/tmp/only-notes.md"), t(300)).unwrap();
        add_at(&file, &notes, Path::new("/tmp/a b.txt"), t(400)).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(blocks(&text).len(), 2, "{text}");
        assert!(text.contains(
            "href=\"file:///tmp/a%20b.txt\" added=\"1970-01-01T00:01:40.000000Z\" modified=\"1970-01-01T00:06:40"
        ));
        assert!(text.contains(
            "name=\"notes\" exec=\"&apos;notes %u&apos;\" modified=\"1970-01-01T00:06:40.000000Z\" count=\"2\""
        ));
        assert!(text.contains("name=\"gedit\""));

        clear(&file, &notes).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(blocks(&text).len(), 1, "the shared bookmark stays for gedit:\n{text}");
        assert!(!text.contains("name=\"notes\"") && !text.contains("only-notes"));
        assert!(text.trim_end().ends_with("</xbel>") && text.contains("<bookmark:applications>"));

        clear(&file, &other).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), EMPTY);
    }

    #[test]
    fn refuses_a_file_that_is_not_xbel() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recently-used.xbel");
        std::fs::write(&file, "garbage").unwrap();
        assert!(add(&file, &app("notes"), Path::new("/x")).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "garbage");
    }
}
