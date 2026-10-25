//! File paths as `file:` URIs and back. Only local paths go either way: a
//! URI with a host (`file://server/share`), and on Windows any path that is
//! not on a drive, gives `None`, so a server can never point ochrepad at a
//! network path (the app also checks links with `is_local_path`).

use std::path::{Component, Path, PathBuf, Prefix};

/// The `file:` URI of an absolute local path.
pub fn file_uri(path: &Path) -> Option<String> {
    let mut out = String::from("file://");
    if cfg!(windows) {
        let mut parts = path.components();
        let drive = match parts.next()? {
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d as char,
                _ => return None,
            },
            _ => return None,
        };
        out.push('/');
        out.push(drive.to_ascii_uppercase());
        out.push(':');
        let mut any = false;
        for c in parts {
            match c {
                Component::RootDir => {}
                Component::Normal(name) => {
                    out.push('/');
                    encode(&name.to_string_lossy(), &mut out);
                    any = true;
                }
                // `..` and `.` are not in absolute paths a user opened.
                _ => return None,
            }
        }
        if !any {
            out.push('/');
        }
    } else {
        if !path.is_absolute() {
            return None;
        }
        let text = path.to_str()?;
        for (i, part) in text.split('/').enumerate() {
            if i > 0 {
                out.push('/');
            }
            encode(part, &mut out);
        }
    }
    Some(out)
}

fn encode(text: &str, out: &mut String) {
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
}

/// The local path a `file:` URI names, if it is one.
pub fn file_path(uri: &str) -> Option<PathBuf> {
    let scheme_end = uri.find(':')?;
    if !uri[..scheme_end].eq_ignore_ascii_case("file") {
        return None;
    }
    let rest = &uri[scheme_end + 1..];
    // No query or fragment in a path.
    let rest = rest.split(['?', '#']).next()?;
    let path = if let Some(after) = rest.strip_prefix("//") {
        let slash = after.find('/')?;
        let host = &after[..slash];
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return None;
        }
        &after[slash..]
    } else if rest.starts_with('/') {
        rest
    } else {
        return None;
    };
    let decoded = decode(path)?;
    if cfg!(windows) {
        // `/C:/dir/file`.
        let p = decoded.strip_prefix('/')?;
        let b = p.as_bytes();
        if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
            return None;
        }
        if b.len() > 2 && b[2] != b'/' {
            return None;
        }
        let path = PathBuf::from(p.replace('/', "\\"));
        local_drive_path(&path).then_some(path)
    } else {
        Some(PathBuf::from(decoded))
    }
}

/// A server's URI spelled as ours are (`file:///c%3A/x` and `file:///c:/x`
/// as `file:///C:/x`), so it finds the document it names; anything else
/// unchanged.
pub fn normalize(uri: &str) -> String {
    file_path(uri)
        .and_then(|p| file_uri(&p))
        .unwrap_or_else(|| uri.to_owned())
}

/// A local path spelled one way: Windows' verbatim form (`\\?\c:\x`, which
/// `canonicalize` gives) as `C:\x`; anything else unchanged.
pub fn plain_path(path: &Path) -> PathBuf {
    file_uri(path)
        .and_then(|u| file_path(&u))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Windows: a path on a drive (not UNC, a device or a verbatim path).
fn local_drive_path(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_))
    )
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    // A NUL is not in a path.
    (!s.contains('\0')).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_uris() {
        if cfg!(windows) {
            let p = Path::new(r"C:\Users\Me\my project\ä#1.rs");
            let uri = file_uri(p).unwrap();
            assert_eq!(uri, "file:///C:/Users/Me/my%20project/%C3%A4%231.rs");
            assert_eq!(file_path(&uri).unwrap(), p);
            assert_eq!(file_uri(Path::new(r"\\?\D:\x")).unwrap(), "file:///D:/x");
            assert_eq!(file_uri(Path::new(r"\\server\share\x")), None);
            assert_eq!(
                file_path("file:///c%3A/dir/a.rs").unwrap(),
                Path::new(r"c:\dir\a.rs")
            );
            assert_eq!(file_path("file:///dir/a.rs"), None, "no drive");
            assert_eq!(
                plain_path(Path::new(r"\\?\c:\a b\x.rs")),
                Path::new(r"C:\a b\x.rs")
            );
            assert_eq!(
                plain_path(Path::new(r"\\server\s\x")),
                Path::new(r"\\server\s\x")
            );
            // rust-analyzer writes the drive in lower case.
            assert_eq!(
                normalize("file:///c:/dir/a%20b.rs"),
                "file:///C:/dir/a%20b.rs"
            );
            assert_eq!(normalize("file:///c%3A/dir/a.rs"), "file:///C:/dir/a.rs");
        } else {
            assert_eq!(normalize("file:///home/%61.rs"), "file:///home/a.rs");
            let p = Path::new("/home/me/my project/ä#1.rs");
            let uri = file_uri(p).unwrap();
            assert_eq!(uri, "file:///home/me/my%20project/%C3%A4%231.rs");
            assert_eq!(file_path(&uri).unwrap(), p);
            assert_eq!(file_uri(Path::new("relative")), None);
        }
        for refused in [
            "file://server/share/x",
            "http://example.com/x",
            "untitled:Untitled-1",
            "file:///x%zz",
            "file:///a%00b",
            "file:relative",
        ] {
            assert_eq!(file_path(refused), None, "{refused}");
            assert_eq!(normalize(refused), refused, "left alone");
        }
        assert!(
            file_path(if cfg!(windows) {
                "file://localhost/C:/x"
            } else {
                "file://localhost/x"
            })
            .is_some()
        );
    }

    #[test]
    fn uris_parse_as_the_protocols() {
        let p = if cfg!(windows) {
            r"C:\a b\[x].rs"
        } else {
            "/a b/[x].rs"
        };
        let uri = file_uri(Path::new(p)).unwrap();
        assert!(uri.parse::<lsp_types::Uri>().is_ok(), "{uri}");
    }
}
