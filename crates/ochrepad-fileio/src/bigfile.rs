//! Files too large to edit (PLAN_v0.3.0 P7, section 3.2): read-only access
//! without reading the file into memory.
//!
//! - A sparse **line index**, the start of every [`LINES_PER_STOP`]th line,
//!   is built on a background thread and published as it grows, so the first
//!   screen shows at once and the line count follows. Finding any line is one
//!   index lookup plus a scan over at most `LINES_PER_STOP - 1` lines. The
//!   index of a 4 GB log of 40 million lines is about 300 KB.
//! - The lines on screen are read through a **memory map** of the file.
//! - Line endings are recognised (`\n`, `\r\n` and a lone `\r` each end a
//!   line) but not changed: offsets are offsets in the file.
//!
//! The file may change while it is open; nothing here may crash because of
//! that. Long passes over the whole file (the index, searches) use
//! positional reads, never the map: on Linux, touching a mapped page after
//! another program truncated the file kills the process (SIGBUS), and those
//! passes take seconds. The viewer calls [`BigFile::refresh`] at the start of
//! every frame, which checks the file's length and re-maps (a shrink reloads
//! everything); the map is then only touched for the few lines drawn in that
//! frame. What remains on Linux is a truncation landing in the moment
//! between that check and those reads. Windows refuses to truncate a mapped
//! file at all (the other program's truncation fails instead).

use std::borrow::Cow;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use encoding_rs::{Encoding, UTF_8};
use memchr::memchr2_iter;
use memmap2::{Mmap, MmapOptions};
use ochrepad_core::LineEnding;
use ochrepad_core::search::ByteSource;

use crate::LoadError;

/// Lines between two entries of the index.
pub const LINES_PER_STOP: u64 = 1024;
/// Bytes the index builder reads at a time.
const INDEX_CHUNK: usize = 4 << 20;
/// Bytes looked at to tell the encoding.
const SNIFF: usize = 64 * 1024;

/// Why a file cannot be shown.
#[derive(Debug)]
pub enum OpenError {
    Load(LoadError),
    /// UTF-16 text: its line breaks are two bytes, so it cannot be indexed
    /// as it is on disk.
    Utf16,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Load(e) => e.fmt(f),
            OpenError::Utf16 => f.write_str(
                "it is UTF-16 text, which the viewer for large files cannot show \
                 (only UTF-8 and single-byte encodings)",
            ),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Load(LoadError::Io(e))
    }
}

/// What [`BigFile::refresh`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    /// Longer than before: the new part is being indexed.
    Grew,
    /// Shorter than before (truncated, or rewritten): everything was
    /// reloaded.
    Reloaded,
}

/// How far the index has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Bytes indexed, of `len`.
    pub scanned: u64,
    pub len: u64,
    /// Lines whose start is known: every line when `done`.
    pub lines: u64,
    pub done: bool,
}

impl Progress {
    /// The number of lines, estimated from the part indexed so far while
    /// the index is still being built.
    pub fn estimated_lines(&self) -> u64 {
        if self.done || self.scanned == 0 {
            return self.lines;
        }
        let per_byte = self.lines as f64 / self.scanned as f64;
        (self.len as f64 * per_byte).max(self.lines as f64) as u64
    }
}

#[derive(Clone, Debug, Default)]
struct Index {
    /// `stops[k]` is where line `k * LINES_PER_STOP` starts.
    stops: Vec<u64>,
    /// Line breaks found so far: lines `0..=breaks` have known starts.
    breaks: u64,
    /// Bytes read so far, from the start of the file.
    scanned: u64,
    /// The builder reached the end of the file (as long as it was then).
    done: bool,
    /// The last byte read was a `\r` at the end of the file, counted as a
    /// line break; a `\n` after it (when the file grows) belongs to it.
    pending_cr: bool,
    /// Line breaks of each kind: CR LF, LF, CR.
    endings: [u64; 3],
    error: Option<String>,
}

/// A large file opened for reading.
pub struct BigFile {
    path: PathBuf,
    file: Arc<File>,
    /// The file mapped as it was at the last refresh (`None` while empty).
    map: Option<Mmap>,
    /// The file's length at the last refresh.
    len: u64,
    encoding: &'static Encoding,
    /// Length of a UTF-8 byte order mark before line 0.
    bom: u64,
    index: Arc<Mutex<Index>>,
    /// Stops the index builder and searches of this file.
    cancel: Arc<AtomicBool>,
    /// Bumped whenever the file is reloaded, so callers drop what they
    /// cached about its lines.
    generation: u64,
}

impl Drop for BigFile {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Reads at `at` until `buf` is full or the file ends; returns the bytes
/// read (0 on an error).
fn read_full_at(file: &File, at: u64, buf: &mut [u8]) -> usize {
    let mut done = 0;
    while done < buf.len() {
        #[cfg(unix)]
        let r = std::os::unix::fs::FileExt::read_at(file, &mut buf[done..], at + done as u64);
        #[cfg(windows)]
        let r = std::os::windows::fs::FileExt::seek_read(file, &mut buf[done..], at + done as u64);
        match r {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    done
}

/// The encoding of a file from its first bytes, and the length of a UTF-8
/// byte order mark.
fn sniff_encoding(head: &[u8]) -> Result<(&'static Encoding, u64), OpenError> {
    if let Some((enc, bom)) = Encoding::for_bom(head) {
        return if enc == UTF_8 {
            Ok((UTF_8, bom as u64))
        } else {
            Err(OpenError::Utf16)
        };
    }
    // UTF-16 without a byte order mark: a NUL in most even or most odd
    // positions (ASCII text stored as two bytes per character).
    let pairs = head.len() / 2;
    if pairs >= 16 {
        let even = head.iter().step_by(2).filter(|&&b| b == 0).count();
        let odd = head.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        if even.max(odd) * 4 > pairs * 3 {
            return Err(OpenError::Utf16);
        }
    }
    match std::str::from_utf8(head) {
        Ok(_) => Ok((UTF_8, 0)),
        // Only a character cut off by the sample's end.
        Err(e) if e.error_len().is_none() => Ok((UTF_8, 0)),
        Err(_) => {
            let mut detector =
                chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
            detector.feed(head, false);
            let guess = detector.guess(None, chardetng::Utf8Detection::Allow);
            // Only encodings where `\n` and `\r` are always line breaks.
            Ok((
                if guess.is_ascii_compatible() {
                    guess
                } else {
                    UTF_8
                },
                0,
            ))
        }
    }
}

/// Line breaks in `bytes` (`\n`, `\r\n` and a lone `\r`, each once). `next`
/// is the byte after `bytes`: a `\r` at the end followed by `\n` is counted
/// with that `\n`, outside.
fn count_breaks(bytes: &[u8], next: Option<u8>) -> u64 {
    let mut n = 0;
    for i in memchr2_iter(b'\n', b'\r', bytes) {
        if bytes[i] == b'\r' && bytes.get(i + 1).copied().or(next) == Some(b'\n') {
            continue;
        }
        n += 1;
    }
    n
}

/// The offset just after the `n`-th line break in `bytes` (`n` ≥ 1).
fn after_breaks(bytes: &[u8], n: u64) -> Option<usize> {
    let mut seen = 0;
    for i in memchr2_iter(b'\n', b'\r', bytes) {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            continue;
        }
        seen += 1;
        if seen == n {
            return Some(i + 1);
        }
    }
    None
}

/// Builds (or continues) the index of `file`, reading `chunk` bytes at a
/// time until the end of the file or `cancel`.
fn build_index(file: &File, index: &Mutex<Index>, cancel: &AtomicBool, chunk: usize) {
    let mut state = lock(index).clone();
    let mut buf = vec![0u8; chunk];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let n = read_full_at(file, state.scanned, &mut buf);
        if n == 0 {
            state.done = true;
            break;
        }
        let bytes = &buf[..n];
        let mut skip_lf = false;
        if std::mem::take(&mut state.pending_cr) && bytes[0] == b'\n' {
            // The `\r` that ended the last read (counted as a line break)
            // was the start of a CR LF.
            state.endings[2] -= 1;
            state.endings[0] += 1;
            skip_lf = true;
            if state.breaks.is_multiple_of(LINES_PER_STOP)
                && state.stops.last() == Some(&state.scanned)
            {
                *state.stops.last_mut().expect("just checked") += 1;
            }
        }
        let base = state.scanned;
        for i in memchr2_iter(b'\n', b'\r', bytes) {
            if i == 0 && skip_lf {
                continue;
            }
            match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => continue,
                b'\r' => {
                    state.endings[2] += 1;
                    // The next read (or the file growing) may start with
                    // the `\n` of a CR LF.
                    state.pending_cr = i + 1 == bytes.len();
                }
                _ if i > 0 && bytes[i - 1] == b'\r' => state.endings[0] += 1,
                _ => state.endings[1] += 1,
            }
            state.breaks += 1;
            if state.breaks.is_multiple_of(LINES_PER_STOP) {
                state.stops.push(base + i as u64 + 1);
            }
        }
        state.scanned = base + bytes.len() as u64;
        publish(index, &state);
    }
    publish(index, &state);
}

/// Copies the builder's progress into the shared index.
fn publish(index: &Mutex<Index>, state: &Index) {
    let mut shared = lock(index);
    let have = shared.stops.len().min(state.stops.len());
    shared.stops.extend_from_slice(&state.stops[have..]);
    shared.breaks = state.breaks;
    shared.scanned = state.scanned;
    shared.pending_cr = state.pending_cr;
    shared.endings = state.endings;
    shared.done = state.done;
}

/// The index, even if a builder thread panicked while holding it (its
/// data is still consistent: every field is written under one lock).
fn lock(index: &Mutex<Index>) -> MutexGuard<'_, Index> {
    index.lock().unwrap_or_else(|e| e.into_inner())
}

impl BigFile {
    /// Opens `path`, maps it and starts indexing it in the background.
    pub fn open(path: &Path) -> Result<BigFile, OpenError> {
        let (file, meta) = crate::open_regular_file(path).map_err(OpenError::Load)?;
        let len = meta.len();
        let mut head = vec![0u8; SNIFF.min(len as usize)];
        let n = read_full_at(&file, 0, &mut head);
        head.truncate(n);
        let (encoding, bom) = sniff_encoding(&head)?;
        let mut big = BigFile {
            path: path.to_path_buf(),
            file: Arc::new(file),
            map: None,
            len: 0,
            encoding,
            bom,
            index: Arc::new(Mutex::new(Index::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
        };
        big.map_to(len)?;
        big.restart_index();
        Ok(big)
    }

    /// Maps the first `len` bytes of the file.
    fn map_to(&mut self, len: u64) -> io::Result<()> {
        self.map = None;
        self.len = 0;
        if len > 0 {
            let size = usize::try_from(len).map_err(io::Error::other)?;
            // SAFETY: the map is read-only and only read through `bytes`,
            // which clamps to `len`. The file can still change underneath
            // (another program writes or truncates it); see the module
            // documentation for how that is kept from crashing.
            let map = unsafe { MmapOptions::new().len(size).map(&*self.file)? };
            self.map = Some(map);
        }
        self.len = len;
        Ok(())
    }

    /// Starts indexing from scratch (on open and after a reload).
    fn restart_index(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.index = Arc::new(Mutex::new(Index {
            stops: vec![self.bom],
            scanned: self.bom,
            ..Index::default()
        }));
        self.spawn_builder();
    }

    fn spawn_builder(&self) {
        let (file, index, cancel) = (self.file.clone(), self.index.clone(), self.cancel.clone());
        let spawned = std::thread::Builder::new()
            .name("ochrepad-index".into())
            .spawn(move || build_index(&file, &index, &cancel, INDEX_CHUNK));
        if let Err(e) = spawned {
            let mut index = lock(&self.index);
            index.error = Some(e.to_string());
            index.done = true;
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The length when last refreshed.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How the file's bytes become text.
    pub fn decoder(&self) -> TextDecoder {
        TextDecoder(self.encoding)
    }

    pub fn has_bom(&self) -> bool {
        self.bom > 0
    }

    /// Changes whenever the file is reloaded.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn progress(&self) -> Progress {
        let index = lock(&self.index);
        Progress {
            scanned: index.scanned.min(self.len),
            len: self.len,
            lines: index.breaks + 1,
            done: index.done,
        }
    }

    /// Why indexing stopped early, if it did.
    pub fn index_error(&self) -> Option<String> {
        lock(&self.index).error.clone()
    }

    /// The most common line ending so far (CR LF on a tie), and whether
    /// there is more than one kind.
    pub fn line_ending(&self) -> (LineEnding, bool) {
        let [crlf, lf, cr] = lock(&self.index).endings;
        let kinds = [crlf, lf, cr].iter().filter(|&&n| n > 0).count();
        let ending = if crlf + lf + cr == 0 {
            crate::default_line_ending()
        } else if crlf >= lf && crlf >= cr {
            LineEnding::CrLf
        } else if lf >= cr {
            LineEnding::Lf
        } else {
            LineEnding::Cr
        };
        (ending, kinds > 1)
    }

    /// Checks the file on disk: a longer file is mapped again and its new
    /// part indexed; a shorter one is reloaded from scratch. Call it before
    /// reading lines in a frame.
    pub fn refresh(&mut self) -> Change {
        let Ok(meta) = self.file.metadata() else {
            return Change::Unchanged;
        };
        let now = meta.len();
        if now == self.len {
            return Change::Unchanged;
        }
        if now < self.len {
            self.reload(now);
            return Change::Reloaded;
        }
        if self.map_to(now).is_err() {
            self.reload(now);
            return Change::Reloaded;
        }
        let finished = lock(&self.index).done;
        if finished {
            lock(&self.index).done = false;
            self.spawn_builder();
        }
        Change::Grew
    }

    /// Reads everything again: after the file shrank (or was rewritten).
    fn reload(&mut self, len: u64) {
        self.cancel.store(true, Ordering::Relaxed);
        let mut head = vec![0u8; SNIFF.min(len as usize)];
        let n = read_full_at(&self.file, 0, &mut head);
        head.truncate(n);
        if let Ok((encoding, bom)) = sniff_encoding(&head) {
            self.encoding = encoding;
            self.bom = bom;
        }
        if self.map_to(len).is_err() {
            self.map = None;
            self.len = 0;
        }
        self.generation += 1;
        self.restart_index();
    }

    /// The bytes in `range` of the map, clamped to the file's length at the
    /// last refresh.
    pub fn bytes(&self, range: Range<u64>) -> &[u8] {
        let Some(map) = &self.map else {
            return &[];
        };
        let end = range.end.min(self.len) as usize;
        let start = (range.start as usize).min(end);
        &map[start..end]
    }

    /// Reads `range` from the file (not the map): for copying a selection,
    /// which may be large.
    pub fn read(&self, range: Range<u64>) -> Vec<u8> {
        let end = range.end.min(self.len);
        let start = range.start.min(end);
        let mut buf = vec![0u8; (end - start) as usize];
        let n = read_full_at(&self.file, start, &mut buf);
        buf.truncate(n);
        buf
    }

    /// Where line `line` (0-based) starts, if the index has reached it.
    pub fn line_start(&self, line: u64) -> Option<u64> {
        let (stop, skip) = {
            let index = lock(&self.index);
            if line > index.breaks {
                return None;
            }
            let k = (line / LINES_PER_STOP) as usize;
            (*index.stops.get(k)?, line % LINES_PER_STOP)
        };
        if skip == 0 {
            return Some(stop);
        }
        let from = stop.min(self.len);
        after_breaks(self.bytes(from..self.len), skip).map(|i| from + i as u64)
    }

    /// Where the text of the line starting at `start` ends (before its line
    /// break, or at the end of the file).
    pub fn line_end(&self, start: u64) -> u64 {
        let bytes = self.bytes(start..self.len);
        match memchr::memchr2(b'\n', b'\r', bytes) {
            Some(i) => start + i as u64,
            None => start + bytes.len() as u64,
        }
    }

    /// The line holding `offset`, if the index has reached it.
    pub fn line_of(&self, offset: u64) -> Option<u64> {
        let (k, stop) = {
            let index = lock(&self.index);
            if offset > index.scanned {
                return None;
            }
            let k = index
                .stops
                .partition_point(|&s| s <= offset)
                .saturating_sub(1);
            (k, *index.stops.get(k)?)
        };
        let from = stop.min(offset);
        let next = self.bytes(offset..offset + 1).first().copied();
        let breaks = count_breaks(self.bytes(from..offset), next);
        Some(k as u64 * LINES_PER_STOP + breaks)
    }

    /// Decodes bytes of the file (invalid sequences become U+FFFD).
    pub fn decode<'a>(&self, bytes: &'a [u8]) -> Cow<'a, str> {
        self.decoder().decode(bytes)
    }

    /// The file for a background search, as it is now: reads stop when
    /// this file is reloaded or closed, or when `stop` is set.
    pub fn source(&self, stop: Arc<AtomicBool>) -> FileSource {
        FileSource {
            file: self.file.clone(),
            len: self.len,
            stops: [self.cancel.clone(), stop],
        }
    }

    /// Line numbers for a background search of this file.
    pub fn line_finder(&self) -> LineFinder {
        LineFinder {
            file: self.file.clone(),
            index: self.index.clone(),
            len: self.len,
            cancel: self.cancel.clone(),
        }
    }

    /// Blocks until the index is complete (tests and measurements).
    pub fn wait_for_index(&self, timeout: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if lock(&self.index).done {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        false
    }
}

/// How a large file's bytes become text: UTF-8, or the legacy encoding it
/// was found to be in. Invalid bytes become U+FFFD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDecoder(&'static Encoding);

impl TextDecoder {
    pub fn is_utf8(self) -> bool {
        self.0 == UTF_8
    }

    pub fn name(self) -> &'static str {
        self.0.name()
    }

    pub fn decode(self, bytes: &[u8]) -> Cow<'_, str> {
        if self.is_utf8() {
            String::from_utf8_lossy(bytes)
        } else {
            self.0.decode_without_bom_handling(bytes).0
        }
    }

    /// The offset in `raw` (a line's bytes) where the character at `col`
    /// of its decoded text starts.
    pub fn byte_of_col(self, raw: &[u8], col: usize) -> usize {
        if self.is_utf8() {
            let (mut decoded, mut at) = (0, 0);
            for chunk in raw.utf8_chunks() {
                let valid = chunk.valid().len();
                if decoded + valid >= col {
                    return at + (col - decoded);
                }
                decoded += valid;
                at += valid;
                if !chunk.invalid().is_empty() {
                    // One U+FFFD (three bytes) stands for the invalid bytes.
                    if decoded + 3 > col {
                        return at;
                    }
                    decoded += 3;
                    at += chunk.invalid().len();
                }
            }
            return at;
        }
        let mut decoder = self.0.new_decoder_without_bom_handling();
        let mut out = String::new();
        for (i, b) in raw.iter().enumerate() {
            if out.len() >= col {
                return i;
            }
            out.reserve(16);
            let _ = decoder.decode_to_string(std::slice::from_ref(b), &mut out, false);
        }
        raw.len()
    }
}

/// Line numbers for a background search (see [`BigFile::line_finder`]),
/// read with positional reads like the search itself.
#[derive(Clone)]
pub struct LineFinder {
    file: Arc<File>,
    index: Arc<Mutex<Index>>,
    len: u64,
    cancel: Arc<AtomicBool>,
}

impl LineFinder {
    /// The line holding `offset`, waiting for the index to get there.
    /// `hint` is an earlier offset and its line (the previous match), to
    /// count from when it is closer than the index's stop. `None` if the
    /// file is closed or reloaded, or `stop` is set, while waiting.
    pub fn line_of(&self, offset: u64, hint: Option<(u64, u64)>, stop: &AtomicBool) -> Option<u64> {
        let (stop_off, stop_line) = loop {
            if stop.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed) {
                return None;
            }
            {
                let index = lock(&self.index);
                if index.scanned >= offset || index.done {
                    let k = index
                        .stops
                        .partition_point(|&s| s <= offset)
                        .saturating_sub(1);
                    break (*index.stops.get(k)?, k as u64 * LINES_PER_STOP);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let (from, line) = match hint {
            Some((o, l)) if o >= stop_off && o <= offset => (o, l),
            _ => (stop_off, stop_line),
        };
        Some(line + self.count_breaks(from, offset))
    }

    /// Line breaks in `from..to` of the file.
    fn count_breaks(&self, from: u64, to: u64) -> u64 {
        let mut buf = vec![0u8; 1 << 20];
        let (mut at, mut breaks) = (from, 0);
        while at < to {
            let want = ((to - at) as usize).min(buf.len() - 1);
            // One byte more: a `\r` at the end may be half of a CR LF.
            let n = read_full_at(&self.file, at, &mut buf[..want + 1]);
            if n == 0 {
                break;
            }
            let used = n.min(want);
            breaks += count_breaks(&buf[..used], buf.get(used).copied().filter(|_| n > used));
            at += used as u64;
        }
        breaks
    }

    /// Reads `range` of the file (clamped to its length when opened).
    pub fn read(&self, range: Range<u64>) -> Vec<u8> {
        let end = range.end.min(self.len);
        let start = range.start.min(end);
        let mut buf = vec![0u8; (end - start) as usize];
        let n = read_full_at(&self.file, start, &mut buf);
        buf.truncate(n);
        buf
    }
}

/// A large file read in pieces by a background search (a [`ByteSource`]).
#[derive(Clone)]
pub struct FileSource {
    file: Arc<File>,
    len: u64,
    stops: [Arc<AtomicBool>; 2],
}

impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, at: u64, buf: &mut [u8]) -> usize {
        if self.stops.iter().any(|s| s.load(Ordering::Relaxed)) || at >= self.len {
            return 0;
        }
        let n = buf.len().min((self.len - at) as usize);
        read_full_at(&self.file, at, &mut buf[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write(text: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    fn open(path: &Path) -> BigFile {
        let f = BigFile::open(path).unwrap();
        assert!(f.wait_for_index(Duration::from_secs(10)));
        f
    }

    /// Every line's text, through the index.
    fn lines(f: &BigFile) -> Vec<String> {
        let n = f.progress().lines;
        (0..n)
            .map(|l| {
                let start = f.line_start(l).unwrap();
                String::from_utf8(f.bytes(start..f.line_end(start)).to_vec()).unwrap()
            })
            .collect()
    }

    #[test]
    fn lines_end_at_lf_crlf_and_cr_and_the_last_may_have_no_break() {
        let (_d, path) = write(b"one\ntwo\r\nthree\rfour");
        let f = open(&path);
        assert_eq!(lines(&f), vec!["one", "two", "three", "four"]);
        assert_eq!(f.line_ending(), (LineEnding::CrLf, true));
        let (_d, path) = write(b"a\nb\n");
        let f = open(&path);
        assert_eq!(lines(&f), vec!["a", "b", ""]);
        assert_eq!(f.line_ending(), (LineEnding::Lf, false));
        let (_d, path) = write(b"");
        let f = open(&path);
        assert_eq!(lines(&f), vec![""]);
        assert!(f.is_empty());
    }

    #[test]
    fn a_utf8_bom_is_skipped_and_utf16_is_refused() {
        let (_d, path) = write(b"\xEF\xBB\xBFhello\nworld");
        let f = open(&path);
        assert!(f.has_bom());
        assert_eq!(lines(&f), vec!["hello", "world"]);
        let (_d, path) = write(b"\xFF\xFEh\0i\0");
        assert!(matches!(BigFile::open(&path), Err(OpenError::Utf16)));
        let utf16: Vec<u8> = "hello world, no byte order mark here\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let (_d, path) = write(&utf16);
        assert!(matches!(BigFile::open(&path), Err(OpenError::Utf16)));
        // A legacy single-byte encoding is decoded per line.
        let (_d, path) = write(b"caf\xE9 cr\xE8me br\xFBl\xE9e na\xEFve d\xE9j\xE0 vu\n");
        let f = open(&path);
        assert!(!f.decoder().is_utf8());
        let start = f.line_start(0).unwrap();
        assert!(
            f.decode(f.bytes(start..f.line_end(start)))
                .starts_with("café crème")
        );
    }

    #[test]
    fn many_lines_through_the_sparse_index() {
        let mut text = String::new();
        let mut starts = Vec::new();
        for i in 0..5000 {
            starts.push(text.len() as u64);
            let ending = ["\n", "\r\n", "\r"][i % 3];
            text.push_str(&format!("line {i}{}{ending}", "x".repeat(i % 17)));
        }
        let (_d, path) = write(text.as_bytes());
        let f = open(&path);
        assert_eq!(f.progress().lines, 5001);
        for line in [0, 1, 1023, 1024, 1025, 2048, 4999] {
            let start = f.line_start(line as u64).unwrap();
            assert_eq!(start, starts[line], "line {line}");
            assert_eq!(f.line_of(start), Some(line as u64));
            let end = f.line_end(start);
            assert_eq!(f.line_of(end), Some(line as u64), "end of {line}");
        }
        assert_eq!(f.line_start(5001), None);
    }

    #[test]
    fn breaks_split_across_reads_are_counted_once() {
        // Tiny reads put a `\r` at the end of almost every read.
        let text = b"a\r\nb\r\n\r\nc\rd\n\re";
        let (_d, path) = write(text);
        let file = File::open(&path).unwrap();
        for chunk in 1..8 {
            let index = Mutex::new(Index {
                stops: vec![0],
                ..Index::default()
            });
            build_index(&file, &index, &AtomicBool::new(false), chunk);
            let index = lock(&index);
            assert!(index.done);
            // CR LF three times, LF once, a lone CR twice.
            assert_eq!(index.breaks, 6, "chunk {chunk}");
            assert_eq!(index.endings, [3, 1, 2], "chunk {chunk}");
        }
    }

    #[test]
    fn growing_files_are_indexed_further() {
        let (_d, path) = write(b"one\ntwo\r");
        let mut f = open(&path);
        assert_eq!(f.progress().lines, 3);
        // The `\r` at the old end turns out to start a CR LF.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, b"\nthree\nfour").unwrap();
        drop(file);
        assert_eq!(f.refresh(), Change::Grew);
        assert!(f.wait_for_index(Duration::from_secs(10)));
        assert_eq!(lines(&f), vec!["one", "two", "three", "four"]);
        assert_eq!(f.refresh(), Change::Unchanged);
    }

    /// Truncating a mapped file: refused on Windows, allowed on Linux, where
    /// reading the map past the new end would kill the process. `refresh`
    /// must notice before anything is read.
    #[test]
    fn a_truncated_file_is_reloaded_without_crashing() {
        let text = "0123456789\n".repeat(50_000);
        let (_d, path) = write(text.as_bytes());
        let mut f = open(&path);
        let shrunk = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_len(25));
        if cfg!(windows) {
            assert!(shrunk.is_err(), "Windows refuses to truncate a mapped file");
            return;
        }
        shrunk.unwrap();
        let generation = f.generation();
        assert_eq!(f.refresh(), Change::Reloaded);
        assert_ne!(f.generation(), generation);
        assert!(f.wait_for_index(Duration::from_secs(10)));
        assert_eq!(lines(&f), vec!["0123456789", "0123456789", "012"]);
        assert_eq!(f.bytes(0..1_000_000).len(), 25);
    }

    #[test]
    fn searching_a_file_source() {
        let text = format!("{}needle\n{}needle", "x\n".repeat(700_000), "y".repeat(10));
        let (_d, path) = write(text.as_bytes());
        let f = open(&path);
        let s = ochrepad_core::Searcher::new_raw(&ochrepad_core::SearchQuery::new(
            "needle",
            Default::default(),
        ))
        .unwrap();
        let src = f.source(Arc::new(AtomicBool::new(false)));
        let found: Vec<u64> = s.matches_in(&src, 0..src.len()).map(|m| m.start).collect();
        assert_eq!(found, vec![1_400_000, 1_400_017]);
        assert_eq!(f.line_of(found[1]), Some(700_001));
        let lines = f.line_finder();
        let go = AtomicBool::new(false);
        assert_eq!(lines.line_of(found[0], None, &go), Some(700_000));
        assert_eq!(
            lines.line_of(found[1], Some((found[0], 700_000)), &go),
            Some(700_001)
        );
        assert_eq!(lines.read(found[0]..found[0] + 6), b"needle");
        // A stopped search reads nothing more.
        let stop = Arc::new(AtomicBool::new(true));
        let src = f.source(stop);
        assert_eq!(s.matches_in(&src, 0..src.len()).count(), 0);
    }
}
