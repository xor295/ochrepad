//! Loading and saving files (PLAN 3.6): encoding, BOM and line-ending
//! detection, normalization to LF, and atomic writes.
//!
//! This is the only crate that looks for `\r`.

use std::borrow::Cow;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use encoding_rs::{EncoderResult, Encoding, UTF_8, UTF_16BE, UTF_16LE};
use ochrepad_core::{Buffer, BufferId, Editor, FileMeta, LineEnding};

pub mod tree;
pub mod worker;

/// How many leading bytes are checked for NUL when deciding a file is binary.
const BINARY_SNIFF_LEN: usize = 8 * 1024;

/// Files larger than this are not opened: the text is held in memory several
/// times over (file bytes, decoded text, rope, undo), and a failed allocation
/// would end the program.
pub const MAX_LOAD_SIZE: u64 = 1024 * 1024 * 1024;

#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    /// A NUL byte in the first 8 KB (and no UTF-16 BOM).
    Binary,
    /// The bytes are not valid in the detected encoding.
    Undecodable {
        encoding: &'static str,
    },
    /// A folder, device, pipe or socket: reading one could block or never
    /// end.
    NotAFile,
    /// Larger than [`MAX_LOAD_SIZE`] (the size in bytes).
    TooLarge(u64),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "could not read the file: {e}"),
            LoadError::Binary => f.write_str("the file looks binary (it contains NUL bytes)"),
            LoadError::Undecodable { encoding } => write!(
                f,
                "the file is not valid {encoding}; refusing to open it rather than replace bytes"
            ),
            LoadError::NotAFile => {
                f.write_str("it is not a regular file (a folder, device or pipe)")
            }
            LoadError::TooLarge(size) => write!(
                f,
                "the file is {} MB; files over {} MB are not opened",
                size / (1024 * 1024),
                MAX_LOAD_SIZE / (1024 * 1024)
            ),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<io::Error> for LoadError {
    fn from(e: io::Error) -> Self {
        LoadError::Io(e)
    }
}

#[derive(Debug)]
pub enum SaveError {
    Io(io::Error),
    /// The buffer has never been saved and no path was given.
    NoPath,
    UnknownEncoding(String),
    /// The text contains a character the file's encoding cannot represent.
    Unencodable {
        encoding: String,
        ch: char,
        line: usize,
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "could not write the file: {e}"),
            SaveError::NoPath => f.write_str("the buffer has no file path"),
            SaveError::UnknownEncoding(e) => write!(f, "cannot save in unknown encoding {e:?}"),
            SaveError::Unencodable { encoding, ch, line } => write!(
                f,
                "{ch:?} (U+{:04X}) on line {line} cannot be saved as {encoding}; \
                 change the encoding or remove the character",
                *ch as u32
            ),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        SaveError::Io(e)
    }
}

/// Decoded file contents: text with LF line endings, plus how it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub meta: FileMeta,
}

/// Reads a whole regular file of at most [`MAX_LOAD_SIZE`] bytes. The file
/// is checked after opening it (on Linux without waiting, so a pipe cannot
/// block the open), so it cannot be swapped for something else in between.
pub fn read_file(path: &Path) -> Result<Vec<u8>, LoadError> {
    read_file_up_to(path, MAX_LOAD_SIZE)
}

/// [`read_file`] with another size limit.
pub fn read_file_up_to(path: &Path, limit: u64) -> Result<Vec<u8>, LoadError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK on Linux: opening a FIFO with no writer returns at once.
        // It changes nothing for regular files.
        options.custom_flags(0o4000);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(LoadError::NotAFile);
    }
    if meta.len() > limit {
        return Err(LoadError::TooLarge(meta.len()));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // At most one byte past the limit, in case the file grew since.
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(LoadError::TooLarge(bytes.len() as u64));
    }
    Ok(bytes)
}

/// Reads and decodes a file.
pub fn load(path: &Path) -> Result<Decoded, LoadError> {
    let bytes = read_file(path)?;
    let mut decoded = decode(&bytes)?;
    decoded.meta.path = Some(path.to_path_buf());
    Ok(decoded)
}

/// The encodings offered for Reopen with Encoding and Save with Encoding:
/// (encoding_rs name, name shown).
pub const ENCODINGS: &[(&str, &str)] = &[
    ("UTF-8", "UTF-8"),
    ("UTF-16LE", "UTF-16 LE"),
    ("UTF-16BE", "UTF-16 BE"),
    ("windows-1252", "Western (Windows-1252)"),
    ("ISO-8859-15", "Western (ISO-8859-15)"),
    ("windows-1250", "Central European (Windows-1250)"),
    ("ISO-8859-2", "Central European (ISO-8859-2)"),
    ("windows-1251", "Cyrillic (Windows-1251)"),
    ("KOI8-R", "Cyrillic (KOI8-R)"),
    ("windows-1253", "Greek (Windows-1253)"),
    ("windows-1254", "Turkish (Windows-1254)"),
    ("Shift_JIS", "Japanese (Shift_JIS)"),
    ("EUC-JP", "Japanese (EUC-JP)"),
    ("GBK", "Chinese Simplified (GBK)"),
    ("Big5", "Chinese Traditional (Big5)"),
    ("EUC-KR", "Korean (EUC-KR)"),
];

/// Reads `path` decoding it as `encoding` (an encoding_rs name) instead of
/// detecting: Reopen with Encoding. A BOM for that encoding is skipped and
/// remembered; bytes that are not valid in it are refused.
pub fn load_as(path: &Path, encoding: &str) -> Result<Decoded, LoadError> {
    let bytes = read_file(path)?;
    let encoding = Encoding::for_label(encoding.as_bytes()).ok_or(LoadError::Undecodable {
        encoding: "unknown",
    })?;
    let bom_len = match Encoding::for_bom(&bytes) {
        Some((e, len)) if e == encoding => len,
        _ => 0,
    };
    let body = &bytes[bom_len..];
    let is_utf16 = encoding == UTF_16LE || encoding == UTF_16BE;
    if !is_utf16 && body[..body.len().min(BINARY_SNIFF_LEN)].contains(&0) {
        return Err(LoadError::Binary);
    }
    let (text, line_ending, mixed) = normalize(decode_strict(encoding, body)?);
    Ok(Decoded {
        text,
        meta: FileMeta {
            path: Some(path.to_path_buf()),
            encoding: encoding.name().to_owned(),
            bom: bom_len > 0,
            line_ending,
            mixed_line_endings: mixed,
        },
    })
}

/// Decodes file bytes (PLAN 3.6 steps 1-3). `meta.path` is left empty.
pub fn decode(bytes: &[u8]) -> Result<Decoded, LoadError> {
    let (encoding, bom_len) = match Encoding::for_bom(bytes) {
        Some((enc, len)) => (enc, len),
        None => (UTF_8, 0),
    };
    let body = &bytes[bom_len..];
    let is_utf16 = encoding == UTF_16LE || encoding == UTF_16BE;
    if !is_utf16 && body[..body.len().min(BINARY_SNIFF_LEN)].contains(&0) {
        return Err(LoadError::Binary);
    }

    let (encoding, text) = if bom_len > 0 {
        (encoding, decode_strict(encoding, body)?)
    } else if let Ok(s) = std::str::from_utf8(body) {
        (UTF_8, s.to_owned())
    } else {
        let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
        detector.feed(body, true);
        let guess = detector.guess(None, chardetng::Utf8Detection::Deny);
        (guess, decode_strict(guess, body)?)
    };

    let (text, line_ending, mixed) = normalize(text);
    Ok(Decoded {
        text,
        meta: FileMeta {
            path: None,
            encoding: encoding.name().to_owned(),
            bom: bom_len > 0,
            line_ending,
            mixed_line_endings: mixed,
        },
    })
}

fn decode_strict(encoding: &'static Encoding, bytes: &[u8]) -> Result<String, LoadError> {
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(Cow::into_owned)
        .ok_or(LoadError::Undecodable {
            encoding: encoding.name(),
        })
}

/// The ending used when a file has none to detect.
pub fn default_line_ending() -> LineEnding {
    if cfg!(windows) {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

/// Converts CRLF and lone CR to LF; returns the dominant original ending and
/// whether more than one kind was present. Ties prefer CRLF, then LF.
fn normalize(text: String) -> (String, LineEnding, bool) {
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [crlf, lf, cr].iter().filter(|&&n| n > 0).count();
    let ending = if crlf + lf + cr == 0 {
        default_line_ending()
    } else if crlf >= lf && crlf >= cr {
        LineEnding::CrLf
    } else if lf >= cr {
        LineEnding::Lf
    } else {
        LineEnding::Cr
    };
    let text = if crlf + cr == 0 {
        text
    } else {
        normalize_line_endings(&text).into_owned()
    };
    (text, ending, kinds > 1)
}

/// Converts CRLF and lone CR to LF. Use this for any text entering a buffer
/// from outside a file load, e.g. the clipboard.
pub fn normalize_line_endings(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Encodes LF-normalized text with the file's encoding, BOM and line ending.
pub fn encode(text: &str, meta: &FileMeta) -> Result<Vec<u8>, SaveError> {
    let encoding = Encoding::for_label(meta.encoding.as_bytes())
        .ok_or_else(|| SaveError::UnknownEncoding(meta.encoding.clone()))?;
    let newline = match meta.line_ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
        LineEnding::Cr => "\r",
    };
    let text: Cow<str> = if newline == "\n" {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.replace('\n', newline))
    };

    // encoding_rs has no UTF-16 encoders (WHATWG), so encode those by hand.
    if encoding == UTF_16LE || encoding == UTF_16BE {
        let le = encoding == UTF_16LE;
        let mut out = Vec::with_capacity(2 + text.len() * 2);
        let units = meta
            .bom
            .then_some(0xFEFFu16)
            .into_iter()
            .chain(text.encode_utf16());
        for u in units {
            out.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        return Ok(out);
    }
    if encoding.output_encoding() != encoding {
        return Err(SaveError::UnknownEncoding(meta.encoding.clone()));
    }

    let mut out = Vec::new();
    if meta.bom && encoding == UTF_8 {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    if encoding == UTF_8 {
        out.extend_from_slice(text.as_bytes());
        return Ok(out);
    }
    let mut encoder = encoding.new_encoder();
    let mut src: &str = &text;
    loop {
        let need = encoder
            .max_buffer_length_from_utf8_without_replacement(src.len())
            .unwrap_or(src.len() * 4 + 16);
        out.reserve(need);
        let (result, read) =
            encoder.encode_from_utf8_to_vec_without_replacement(src, &mut out, true);
        match result {
            EncoderResult::InputEmpty => return Ok(out),
            EncoderResult::OutputFull => src = &src[read..],
            EncoderResult::Unmappable(ch) => {
                let consumed = text.len() - src.len() + read;
                let line = text[..consumed].matches(newline).count() + 1;
                return Err(SaveError::Unencodable {
                    encoding: encoding.name().to_owned(),
                    ch,
                    line,
                });
            }
        }
    }
}

/// The different characters of LF-normalized `text` that `encoding` cannot
/// store, each with the first line (1-based) it is on, at most `limit` of
/// them. Empty for UTF-8, UTF-16 and an unknown encoding.
pub fn unencodable(text: &str, encoding: &str, limit: usize) -> Vec<(char, usize)> {
    let Some(encoding) = Encoding::for_label(encoding.as_bytes()) else {
        return Vec::new();
    };
    if encoding == UTF_8 || encoding == UTF_16LE || encoding == UTF_16BE {
        return Vec::new();
    }
    let mut known: std::collections::HashMap<char, bool> = Default::default();
    let mut found = Vec::new();
    let mut buf = [0u8; 4];
    let mut out = Vec::with_capacity(16);
    for (i, line) in text.split('\n').enumerate() {
        for ch in line.chars().filter(|c| !c.is_ascii()) {
            let fits = *known.entry(ch).or_insert_with(|| {
                out.clear();
                let (result, _) = encoding
                    .new_encoder()
                    .encode_from_utf8_to_vec_without_replacement(
                        ch.encode_utf8(&mut buf),
                        &mut out,
                        true,
                    );
                matches!(result, EncoderResult::InputEmpty)
            });
            if !fits {
                known.insert(ch, true);
                found.push((ch, i + 1));
                if found.len() >= limit {
                    return found;
                }
            }
        }
    }
    found
}

/// Encodes and atomically writes `text` to `path`. Nothing is written if
/// encoding fails.
pub fn save(path: &Path, text: &str, meta: &FileMeta) -> Result<(), SaveError> {
    let bytes = encode(text, meta)?;
    write_user_file(path, &bytes)?;
    Ok(())
}

/// Writes `bytes` to a temp file in the target's directory, syncs it, then
/// renames it over the target. A symlinked target is resolved first so the
/// link itself survives. The target's permissions are carried over.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_file(path, bytes, false)
}

/// Writes a user's file like [`write_atomic`], but a new file gets the
/// usual permissions (on Linux `0666` less the umask) instead of the temp
/// file's private `0600`, which suits only ochrepad's own backups and
/// settings. On Windows the file's "downloaded from the internet" mark
/// (its `Zone.Identifier` stream) is kept, so programs still warn before
/// running a downloaded script that was edited here.
pub fn write_user_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_file(path, bytes, true)
}

fn replace_file(path: &Path, bytes: &[u8], user_file: bool) -> io::Result<()> {
    let target: PathBuf = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut tmp = tempfile::Builder::new()
        .prefix(".ochrepad-")
        .suffix(".tmp")
        .tempfile_in(&dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    match std::fs::metadata(&target) {
        Ok(m) => std::fs::set_permissions(tmp.path(), m.permissions())?,
        Err(_) if user_file => set_new_file_permissions(tmp.path())?,
        Err(_) => {}
    }
    if user_file && cfg!(windows) {
        // Best effort: a missing stream or a file system without streams is
        // fine.
        let stream = |p: &Path| PathBuf::from(format!("{}:Zone.Identifier", p.display()));
        if let Ok(mark) = std::fs::read(stream(&target)) {
            let _ = std::fs::write(stream(tmp.path()), mark);
        }
    }
    tmp.persist(&target).map_err(|e| e.error)?;
    Ok(())
}

/// Gives a new file the permissions `File::create` would: `0666` less the
/// process's umask (read from `/proc`, else the usual `022`).
#[cfg(unix)]
fn set_new_file_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let umask = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Umask:"))
                .and_then(|v| u32::from_str_radix(v.trim(), 8).ok())
        })
        .unwrap_or(0o022);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666 & !umask))
}

#[cfg(not(unix))]
fn set_new_file_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The per-user configuration directory (settings, macros, session),
/// e.g. `%APPDATA%\ochrepad\config` on Windows.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ochrepad").map(|d| d.config_dir().to_path_buf())
}

/// The per-user data directory (backup snapshots), e.g.
/// `%APPDATA%\ochrepad\data` on Windows.
pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ochrepad").map(|d| d.data_dir().to_path_buf())
}

/// Reads a JSON file; `Ok(None)` if it does not exist.
pub fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes `value` as pretty JSON, atomically, creating the directory.
pub fn save_json<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Loads a file into a new buffer.
pub fn open_into(editor: &mut Editor, path: &Path) -> Result<BufferId, LoadError> {
    let Decoded { text, meta } = load(path)?;
    Ok(editor.add_buffer(&text, meta))
}

/// Saves a buffer to its own path and marks it clean.
pub fn save_buffer(buffer: &mut Buffer) -> Result<(), SaveError> {
    let path = buffer.meta.path.clone().ok_or(SaveError::NoPath)?;
    save(&path, &buffer.doc().to_string(), &buffer.meta)?;
    buffer.mark_saved();
    Ok(())
}

/// Saves a buffer to a new path, keeping its encoding, BOM and line ending;
/// on success the buffer takes the new path and is marked clean.
pub fn save_buffer_as(buffer: &mut Buffer, path: &Path) -> Result<(), SaveError> {
    save(path, &buffer.doc().to_string(), &buffer.meta)?;
    buffer.meta.path = Some(path.to_path_buf());
    buffer.mark_saved();
    Ok(())
}
