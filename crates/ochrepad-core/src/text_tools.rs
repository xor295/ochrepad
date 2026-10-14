//! Text tools (PLAN_v0.3.0 P4): JSON format, minify and validate with our own
//! tokenizer, Base64 and URL (percent) encoding, and MD5, SHA-1 and SHA-256
//! hashes.
//!
//! The editing tools work on every non-empty selection, or on the whole
//! document when nothing is selected; each run is one undo step. Without a
//! selection, blank space at the start and end of the document is left out
//! (a final line break stays where it is). Text is encoded and hashed as
//! UTF-8 with the document's line endings, so a whole UTF-8 document hashes
//! like its file; decoded line breaks become the editor's.

use std::ops::Range;

use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::action::{Action, ActionError};
use crate::buffer::{Indent, LineEnding};
use crate::changeset::{Bias, ChangeSet, Edit};
use crate::history::EditKind;
use crate::position::{ByteOffset, byte_to_visual_col};
use crate::resolve::Resolved;
use crate::selection::{Selection, SelectionSet};
use crate::text::Document;

/// A text encoding for [`Action::Encode`] and [`Action::Decode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    /// Base64 (RFC 4648) with the standard alphabet and `=` padding.
    /// Decoding also takes the URL-safe alphabet (`-`, `_`), missing
    /// padding, and spaces or line breaks anywhere.
    Base64,
    /// Percent-encoding (RFC 3986): every byte but `A`–`Z`, `a`–`z`,
    /// `0`–`9`, `-`, `.`, `_` and `~` becomes `%XX`. Decoding leaves `+`
    /// alone.
    Url,
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::Base64 => "Base64",
            Codec::Url => "URL encoding",
        }
    }

    /// Encodes the UTF-8 bytes of `text`.
    pub fn encode(self, text: &str) -> String {
        match self {
            Codec::Base64 => base64_encode(text.as_bytes()),
            Codec::Url => url_encode(text.as_bytes()),
        }
    }

    /// Decodes `text`; the bytes must be UTF-8 text.
    pub fn decode(self, text: &str) -> Result<String, String> {
        let bytes = match self {
            Codec::Base64 => base64_decode(text)?,
            Codec::Url => url_decode(text)?,
        };
        String::from_utf8(bytes).map_err(|_| "the decoded bytes are not UTF-8 text".to_owned())
    }
}

// ---------------------------------------------------------------------------
// JSON

/// Where a JSON text stops being valid, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// Byte offset into the text that was read.
    pub offset: usize,
    pub message: String,
}

/// Where the JSON in a document stops being valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidJson {
    pub at: ByteOffset,
    /// Counted from 1, as in the status bar.
    pub line: usize,
    /// The visual column, counted from 1, as in the status bar.
    pub column: usize,
    pub message: String,
}

/// Whitespace between JSON tokens (RFC 8259).
const JSON_SPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// One member or element per line, each level indented by `indent` (a tab,
/// or `indent.width` spaces). Key order, number text and strings are kept
/// exactly; only the whitespace between tokens changes.
pub fn format_json(text: &str, indent: Indent) -> Result<String, JsonError> {
    let unit = indent_unit(indent);
    run_json(
        text,
        Style::Pretty {
            unit: &unit,
            base: "",
        },
        &mut [],
    )
}

/// The JSON with no whitespace between its tokens.
pub fn minify_json(text: &str) -> Result<String, JsonError> {
    run_json(text, Style::Minify, &mut [])
}

/// Checks that `text` is one JSON value (with blank space around it).
pub fn validate_json(text: &str) -> Result<(), JsonError> {
    run_json(text, Style::Check, &mut []).map(drop)
}

fn indent_unit(indent: Indent) -> String {
    if indent.use_spaces {
        " ".repeat(indent.width.max(1))
    } else {
        "\t".to_owned()
    }
}

/// How [`run_json`] writes the JSON back out.
#[derive(Clone, Copy)]
enum Style<'a> {
    /// Check only; write nothing.
    Check,
    /// No whitespace between tokens.
    Minify,
    /// One member or element per line, indented by `unit` per level after
    /// `base` (the indentation of the line the JSON starts on).
    Pretty { unit: &'a str, base: &'a str },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// A value: at the top, after `:`, or after `,` in an array.
    Value,
    /// Right after `[`: a value or `]`.
    FirstElement,
    /// Right after `{`: a key or `}`.
    FirstKey,
    /// After `,` in an object.
    Key,
    Colon,
    /// After a value inside a container.
    CommaOrClose,
    /// After the top-level value: only whitespace.
    End,
}

/// Writes the output and moves positions from the input to the output.
struct Writer<'a, 'p> {
    out: String,
    style: Style<'a>,
    /// Sorted input offsets, replaced by output offsets as tokens pass.
    positions: &'p mut [usize],
    next: usize,
    /// Output offset right after the last token.
    after_last: usize,
    /// A line break is due before the next member or element.
    break_due: bool,
}

impl Writer<'_, '_> {
    /// Copies the token `text[start..end]`. Positions in the whitespace
    /// before it stay after the previous token; positions inside it keep
    /// their place in it.
    fn token(&mut self, token: &str, start: usize) {
        if matches!(self.style, Style::Check) {
            return;
        }
        while let Some(p) = self.positions.get_mut(self.next).filter(|p| **p < start) {
            *p = self.after_last;
            self.next += 1;
        }
        let out_start = self.out.len();
        self.out.push_str(token);
        let end = start + token.len();
        while let Some(p) = self.positions.get_mut(self.next).filter(|p| **p < end) {
            *p = out_start + (*p - start);
            self.next += 1;
        }
        self.after_last = self.out.len();
    }

    /// A line break and the indentation for `depth`, when pretty-printing.
    fn line_break(&mut self, depth: usize) {
        if let Style::Pretty { unit, base } = self.style {
            self.out.push('\n');
            self.out.push_str(base);
            for _ in 0..depth {
                self.out.push_str(unit);
            }
        }
    }

    /// Starts a member or element: on a new line if one is due.
    fn element(&mut self, depth: usize) {
        if std::mem::take(&mut self.break_due) {
            self.line_break(depth);
        }
    }

    fn space(&mut self) {
        if let Style::Pretty { .. } = self.style {
            self.out.push(' ');
        }
    }

    /// Positions after the last token go to the end of the output.
    fn finish(self) -> String {
        if !matches!(self.style, Style::Check) {
            for p in &mut self.positions[self.next..] {
                *p = self.out.len();
            }
        }
        self.out
    }
}

/// What is at `i`, for messages.
fn found(text: &str, i: usize) -> String {
    text.get(i..)
        .and_then(|s| s.chars().next())
        .map_or_else(|| "the end".to_owned(), shown)
}

/// A character for messages: quoted, or its code point if invisible.
fn shown(c: char) -> String {
    if c.is_control() {
        format!("U+{:04X}", c as u32)
    } else {
        format!("'{c}'")
    }
}

fn json_error(offset: usize, message: impl Into<String>) -> JsonError {
    JsonError {
        offset,
        message: message.into(),
    }
}

/// Reads one JSON value and writes it back in `style`. `positions` (sorted
/// offsets into `text`) are replaced by where they end up in the output.
/// Iterative, so deep nesting cannot overflow the stack.
fn run_json(text: &str, style: Style, positions: &mut [usize]) -> Result<String, JsonError> {
    let b = text.as_bytes();
    let capacity = match style {
        Style::Check => 0,
        Style::Minify => text.len(),
        Style::Pretty { .. } => text.len() + text.len() / 2,
    };
    let mut w = Writer {
        out: String::with_capacity(capacity),
        style,
        positions,
        next: 0,
        after_last: 0,
        break_due: false,
    };
    // Open containers: true for an object.
    let mut stack: Vec<bool> = Vec::new();
    let mut expect = Expect::Value;
    let skip = |mut i: usize| {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    };
    let after_value = |stack: &[bool]| {
        if stack.is_empty() {
            Expect::End
        } else {
            Expect::CommaOrClose
        }
    };
    let mut i = skip(0);
    while i < b.len() {
        let c = b[i];
        match expect {
            Expect::End => {
                return Err(json_error(
                    i,
                    format!("unexpected {} after the end of the JSON", found(text, i)),
                ));
            }
            Expect::Colon => {
                if c != b':' {
                    return Err(json_error(
                        i,
                        format!("expected ':' after the key but found {}", found(text, i)),
                    ));
                }
                w.token(":", i);
                w.space();
                expect = Expect::Value;
                i += 1;
            }
            Expect::CommaOrClose => {
                let object = stack.last() == Some(&true);
                match c {
                    b',' => {
                        w.token(",", i);
                        w.break_due = true;
                        expect = if object { Expect::Key } else { Expect::Value };
                    }
                    b'}' if object => {
                        stack.pop();
                        w.line_break(stack.len());
                        w.token("}", i);
                        expect = after_value(&stack);
                    }
                    b']' if !object => {
                        stack.pop();
                        w.line_break(stack.len());
                        w.token("]", i);
                        expect = after_value(&stack);
                    }
                    _ => {
                        return Err(json_error(
                            i,
                            format!(
                                "expected ',' or '{}' but found {}",
                                if object { '}' } else { ']' },
                                found(text, i)
                            ),
                        ));
                    }
                }
                i += 1;
            }
            Expect::FirstKey if c == b'}' => {
                stack.pop();
                w.break_due = false;
                w.token("}", i);
                expect = after_value(&stack);
                i += 1;
            }
            Expect::FirstElement if c == b']' => {
                stack.pop();
                w.break_due = false;
                w.token("]", i);
                expect = after_value(&stack);
                i += 1;
            }
            Expect::FirstKey | Expect::Key => {
                if c == b'"' {
                    let end = scan_string(text, i)?;
                    w.element(stack.len());
                    w.token(&text[i..end], i);
                    expect = Expect::Colon;
                    i = end;
                } else {
                    let wanted = if expect == Expect::FirstKey {
                        "a key in quotes or '}'"
                    } else {
                        "a key in quotes"
                    };
                    return Err(json_error(
                        i,
                        format!("expected {wanted} but found {}", found(text, i)),
                    ));
                }
            }
            Expect::Value | Expect::FirstElement => {
                let end = match c {
                    b'{' | b'[' => i + 1,
                    b'"' => scan_string(text, i)?,
                    b'-' | b'0'..=b'9' => scan_number(text, i)?,
                    _ => ["true", "false", "null"]
                        .into_iter()
                        .find(|lit| text[i..].starts_with(lit))
                        .map(|lit| i + lit.len())
                        .ok_or_else(|| {
                            json_error(i, format!("expected a value but found {}", found(text, i)))
                        })?,
                };
                w.element(stack.len());
                w.token(&text[i..end], i);
                match c {
                    b'{' => {
                        stack.push(true);
                        w.break_due = true;
                        expect = Expect::FirstKey;
                    }
                    b'[' => {
                        stack.push(false);
                        w.break_due = true;
                        expect = Expect::FirstElement;
                    }
                    _ => expect = after_value(&stack),
                }
                i = end;
            }
        }
        i = skip(i);
    }
    if expect != Expect::End {
        let message = match expect {
            Expect::Value | Expect::FirstElement if stack.is_empty() => {
                "expected a value but found the end".to_owned()
            }
            _ => format!(
                "the JSON ends before its {} is closed",
                if stack.last() == Some(&true) {
                    "object"
                } else {
                    "array"
                }
            ),
        };
        return Err(json_error(b.len(), message));
    }
    Ok(w.finish())
}

/// The end of the string starting with the quote at `start`.
fn scan_string(text: &str, start: usize) -> Result<usize, JsonError> {
    let b = text.as_bytes();
    let mut j = start + 1;
    while j < b.len() {
        match b[j] {
            b'"' => return Ok(j + 1),
            b'\\' => match b.get(j + 1) {
                Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => j += 2,
                Some(b'u') => {
                    if b.len() >= j + 6 && b[j + 2..j + 6].iter().all(u8::is_ascii_hexdigit) {
                        j += 6;
                    } else {
                        return Err(json_error(j, "\\u must be followed by four hex digits"));
                    }
                }
                _ => return Err(json_error(j, "invalid escape sequence in a string")),
            },
            b'\n' | b'\r' => {
                return Err(json_error(
                    j,
                    "a string cannot contain a line break (write \\n)",
                ));
            }
            c if c < 0x20 => {
                return Err(json_error(
                    j,
                    format!("a string cannot contain the control character U+{c:04X}"),
                ));
            }
            _ => j += 1,
        }
    }
    Err(json_error(start, "this string has no closing quote"))
}

/// The end of the number starting at `start` (a `-` or a digit).
fn scan_number(text: &str, start: usize) -> Result<usize, JsonError> {
    let b = text.as_bytes();
    let digits = |mut j: usize| {
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        j
    };
    let mut j = start;
    if b[j] == b'-' {
        j += 1;
    }
    match b.get(j) {
        Some(b'0') => {
            j += 1;
            if b.get(j).is_some_and(u8::is_ascii_digit) {
                return Err(json_error(j, "a number cannot have a leading zero"));
            }
        }
        Some(c) if c.is_ascii_digit() => j = digits(j),
        _ => return Err(json_error(j, "expected digits after '-'")),
    }
    if b.get(j) == Some(&b'.') {
        let k = digits(j + 1);
        if k == j + 1 {
            return Err(json_error(k, "expected digits after the decimal point"));
        }
        j = k;
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        j += 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let k = digits(j);
        if k == j {
            return Err(json_error(k, "expected digits in the exponent"));
        }
        j = k;
    }
    Ok(j)
}

// ---------------------------------------------------------------------------
// Base64 and percent-encoding

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 with the standard alphabet and `=` padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let at = |i: usize| chunk.get(i).copied().unwrap_or(0) as u32;
        let n = (at(0) << 16) | (at(1) << 8) | at(2);
        let sextet = |shift: u32| BASE64[(n >> shift) as usize & 63] as char;
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

/// Decodes Base64 in the standard or URL-safe alphabet. Spaces and line
/// breaks are skipped and the `=` padding may be left out.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 2);
    let (mut acc, mut n, mut padding) = (0u32, 0usize, 0usize);
    for c in text.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => {
                padding += 1;
                continue;
            }
            ' ' | '\t' | '\n' | '\r' => continue,
            _ => return Err(format!("{} is not a Base64 character", shown(c))),
        };
        if padding > 0 {
            return Err("there is text after the '=' padding".to_owned());
        }
        acc = (acc << 6) | v;
        n += 1;
        if n == 4 {
            out.extend([(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]);
            (acc, n) = (0, 0);
        }
    }
    match n {
        0 if padding == 0 => {}
        2 if padding <= 2 => out.push((acc >> 4) as u8),
        3 if padding <= 1 => out.extend([(acc >> 10) as u8, (acc >> 2) as u8]),
        1 => return Err("the text is cut short: one character too many or too few".to_owned()),
        _ => return Err("the '=' padding does not fit the text".to_owned()),
    }
    Ok(out)
}

/// Percent-encodes every byte except the RFC 3986 unreserved characters.
pub fn url_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
    out
}

/// Decodes `%XX` escapes; everything else, `+` included, is kept.
pub fn url_decode(text: &str) -> Result<Vec<u8>, String> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let hex = |j: usize| b.get(j).and_then(|&x| (x as char).to_digit(16));
        match (hex(i + 1), hex(i + 2)) {
            (Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            _ => {
                let bad: String = text[i..].chars().take(3).collect();
                return Err(format!(
                    "\"{bad}\" is not an escape: '%' must be followed by two hex digits"
                ));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Hashes

/// Hashes of some text, as lowercase hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hashes {
    /// Bytes hashed.
    pub bytes: usize,
    /// Line breaks in the text (written with the document's line ending).
    pub line_breaks: usize,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

/// Hashes the UTF-8 bytes of `text`, with `ending` for every line break.
pub fn hashes(text: &str, ending: LineEnding) -> Hashes {
    let newline = ending_str(ending).as_bytes();
    let (mut md5, mut sha1, mut sha256) = (Md5::new(), Sha1::new(), Sha256::new());
    let (mut bytes, mut line_breaks) = (0, 0);
    let mut feed = |data: &[u8]| {
        md5.update(data);
        sha1.update(data);
        sha256.update(data);
        bytes += data.len();
    };
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            feed(newline);
            line_breaks += 1;
        }
        feed(line.as_bytes());
    }
    let hex = |digest: &[u8]| digest.iter().map(|b| format!("{b:02x}")).collect();
    Hashes {
        bytes,
        line_breaks,
        md5: hex(&md5.finalize()),
        sha1: hex(&sha1.finalize()),
        sha256: hex(&sha256.finalize()),
    }
}

/// The hash tool's text: the non-empty selections joined by line breaks
/// (as a copy of them would be), or the whole document when nothing is
/// selected. The flag says it was the whole document.
pub fn hash_selections(doc: &Document, sels: &SelectionSet, ending: LineEnding) -> (Hashes, bool) {
    let whole = nothing_selected(sels);
    let text = if whole {
        doc.to_string()
    } else {
        sels.ranges()
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| doc.slice(s.range()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    (hashes(&text, ending), whole)
}

fn ending_str(ending: LineEnding) -> &'static str {
    match ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
        LineEnding::Cr => "\r",
    }
}

// ---------------------------------------------------------------------------
// The actions

fn nothing_selected(sels: &SelectionSet) -> bool {
    sels.ranges().iter().all(Selection::is_empty)
}

/// `r` without JSON whitespace at either end.
fn trimmed(doc: &Document, r: Range<ByteOffset>) -> (Range<ByteOffset>, String) {
    let text = doc.slice(r.clone());
    let start = r.start.0 + (text.len() - text.trim_start_matches(JSON_SPACE).len());
    let body = text.trim_matches(JSON_SPACE).to_owned();
    (ByteOffset(start)..ByteOffset(start + body.len()), body)
}

/// What the tools work on: every non-empty selection, or the document
/// without the blank space at its ends. With `trim`, selections lose
/// theirs too (JSON ignores it).
fn targets(doc: &Document, sels: &SelectionSet, trim: bool) -> Vec<(Range<ByteOffset>, String)> {
    if nothing_selected(sels) {
        return vec![trimmed(doc, ByteOffset(0)..doc.len())];
    }
    sels.ranges()
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| {
            if trim {
                trimmed(doc, s.range())
            } else {
                (s.range(), doc.slice(s.range()))
            }
        })
        .collect()
}

/// The leading spaces and tabs of the line holding `pos`.
fn line_indent(doc: &Document, pos: ByteOffset) -> String {
    doc.line_text(doc.line_of(pos))
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn invalid_json(doc: &Document, start: ByteOffset, e: JsonError, tab_width: usize) -> ActionError {
    let at = ByteOffset(start.0 + e.offset);
    ActionError::InvalidJson(Box::new(InvalidJson {
        at,
        line: doc.line_of(at) + 1,
        column: byte_to_visual_col(doc, at, tab_width).0 + 1,
        message: e.message,
    }))
}

/// Checks the JSON in each selection, or the document when nothing is
/// selected (Text Tools > Validate JSON). Not an edit.
pub fn validate_json_in(
    doc: &Document,
    sels: &SelectionSet,
    tab_width: usize,
) -> Result<(), ActionError> {
    for (r, text) in targets(doc, sels, true) {
        run_json(&text, Style::Check, &mut [])
            .map_err(|e| invalid_json(doc, r.start, e, tab_width))?;
    }
    Ok(())
}

/// Resolves the text-tool actions, which can fail (`Editor` routes them
/// here). `indent` is the document's.
pub(crate) fn resolve(
    action: &Action,
    doc: &Document,
    sels: &SelectionSet,
    indent: Indent,
    ending: LineEnding,
) -> Result<Resolved, ActionError> {
    match action {
        Action::FormatJson { indent: chosen } => {
            let unit = indent_unit(chosen.unwrap_or(indent));
            json(doc, sels, Some(&unit), indent.width)
        }
        Action::MinifyJson => json(doc, sels, None, indent.width),
        Action::Encode(codec) => recode(doc, sels, |text| {
            let text = match ending {
                LineEnding::Lf => text.to_owned(),
                other => text.replace('\n', ending_str(other)),
            };
            Ok(codec.encode(&text))
        }),
        Action::Decode(codec) => {
            let what = if nothing_selected(sels) {
                "the document"
            } else {
                "the selection"
            };
            recode(doc, sels, |text| {
                let decoded = codec.decode(text).map_err(|e| {
                    ActionError::Decode(format!("{what} is not valid {}: {e}", codec.name()))
                })?;
                // Line breaks become the editor's (LF).
                Ok(if decoded.contains('\r') {
                    decoded.replace("\r\n", "\n").replace('\r', "\n")
                } else {
                    decoded
                })
            })
        }
        _ => Ok(Resolved::Selections(sels.clone())),
    }
}

/// Formats (`unit` is the indent per level) or minifies the JSON of every
/// target. Without a selection, carets stay on the token they were on.
fn json(
    doc: &Document,
    sels: &SelectionSet,
    unit: Option<&str>,
    tab_width: usize,
) -> Result<Resolved, ActionError> {
    let whole = nothing_selected(sels);
    let mut edits = Vec::new();
    // (selection index, position after the edit) for carets inside the JSON.
    let mut moved: Vec<(usize, ByteOffset)> = Vec::new();
    for (r, text) in targets(doc, sels, true) {
        let base = unit.map(|_| line_indent(doc, r.start)).unwrap_or_default();
        let style = match unit {
            Some(unit) => Style::Pretty { unit, base: &base },
            None => Style::Minify,
        };
        let mut inside: Vec<(usize, usize)> = if whole {
            sels.ranges()
                .iter()
                .enumerate()
                .filter(|(_, s)| r.start <= s.head && s.head <= r.end)
                .map(|(i, s)| (s.head.0 - r.start.0, i))
                .collect()
        } else {
            Vec::new()
        };
        inside.sort_unstable();
        let mut positions: Vec<usize> = inside.iter().map(|(p, _)| *p).collect();
        let out = run_json(&text, style, &mut positions)
            .map_err(|e| invalid_json(doc, r.start, e, tab_width))?;
        // Nothing before this target changes (there is one without a
        // selection), so its output starts where it did.
        moved.extend(
            inside
                .iter()
                .zip(&positions)
                .map(|((_, i), p)| (*i, ByteOffset(r.start.0 + p))),
        );
        if out != text {
            edits.push(Edit::new(r.start, r.end, out));
        }
    }
    let changes = ChangeSet::new(doc.len(), edits);
    if changes.is_empty() {
        return Ok(Resolved::Selections(sels.clone()));
    }
    let after = if whole {
        let ranges = sels
            .ranges()
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let pos = moved
                    .iter()
                    .find(|(j, _)| *j == i)
                    .map_or_else(|| changes.map_position(s.head, Bias::Before), |m| m.1);
                Selection::caret(pos)
            })
            .collect();
        SelectionSet::new(ranges, sels.primary_index())
    } else {
        cover_new_text(sels, &changes)
    };
    Ok(Resolved::Edit {
        changes,
        after,
        kind: EditKind::Other,
    })
}

/// Replaces every target with `f` of its text.
fn recode(
    doc: &Document,
    sels: &SelectionSet,
    f: impl Fn(&str) -> Result<String, ActionError>,
) -> Result<Resolved, ActionError> {
    let mut edits = Vec::new();
    for (r, text) in targets(doc, sels, false) {
        let new = f(&text)?;
        if new != text {
            edits.push(Edit::new(r.start, r.end, new));
        }
    }
    let changes = ChangeSet::new(doc.len(), edits);
    if changes.is_empty() {
        return Ok(Resolved::Selections(sels.clone()));
    }
    Ok(Resolved::Edit {
        after: cover_new_text(sels, &changes),
        changes,
        kind: EditKind::Other,
    })
}

/// Each selection covers what its text became; carets map through.
fn cover_new_text(sels: &SelectionSet, changes: &ChangeSet) -> SelectionSet {
    sels.map(|s| {
        if s.is_empty() {
            return Selection::caret(changes.map_position(s.head, Bias::Before));
        }
        let start = changes.map_position(s.start(), Bias::Before);
        let end = changes.map_position(s.end(), Bias::After);
        if s.head < s.anchor {
            Selection::new(end, start)
        } else {
            Selection::new(start, end)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACES4: Indent = Indent {
        use_spaces: true,
        width: 4,
    };

    #[test]
    fn formats_nested_json_keeping_order_numbers_and_strings() {
        let input = r#"{"z":1,"a":[true,null,{"c":"d"}],"e":{},"f":[],"n":-0.0,"big":1E+10,"s":"caf\u00e9 \"q\" é 漢\n"}"#;
        let expected = r#"{
    "z": 1,
    "a": [
        true,
        null,
        {
            "c": "d"
        }
    ],
    "e": {},
    "f": [],
    "n": -0.0,
    "big": 1E+10,
    "s": "caf\u00e9 \"q\" é 漢\n"
}"#;
        assert_eq!(format_json(input, SPACES4).unwrap(), expected);
        assert_eq!(minify_json(expected).unwrap(), input);
        let tabs = format_json("[1,[2]]", Indent::default()).unwrap();
        assert_eq!(tabs, "[\n\t1,\n\t[\n\t\t2\n\t]\n]");
    }

    #[test]
    fn scalars_and_empty_containers_are_json_too() {
        for s in [
            "0", "-1.5e-3", "\"\"", "true", "false", "null", "{}", "[]", " [ ] ",
        ] {
            assert!(validate_json(s).is_ok(), "{s}");
        }
        assert_eq!(format_json(" {} ", SPACES4).unwrap(), "{}");
        assert_eq!(minify_json("[ 1 , { } ]").unwrap(), "[1,{}]");
    }

    #[test]
    fn invalid_json_says_where_and_why() {
        let cases: &[(&str, usize, &str)] = &[
            ("", 0, "expected a value but found the end"),
            ("{\"a\":1,}", 7, "expected a key in quotes but found '}'"),
            ("[1,]", 3, "expected a value but found ']'"),
            ("[1 2]", 3, "expected ',' or ']' but found '2'"),
            ("{\"a\" 1}", 5, "expected ':' after the key but found '1'"),
            ("{1:2}", 1, "expected a key in quotes or '}' but found '1'"),
            ("{} x", 3, "unexpected 'x' after the end of the JSON"),
            ("[1", 2, "the JSON ends before its array is closed"),
            ("{\"a\":[", 6, "the JSON ends before its array is closed"),
            ("\"abc", 0, "this string has no closing quote"),
            (
                "\"a\nb\"",
                2,
                "a string cannot contain a line break (write \\n)",
            ),
            ("\"\\x\"", 1, "invalid escape sequence in a string"),
            ("\"\\u12\"", 1, "\\u must be followed by four hex digits"),
            ("01", 1, "a number cannot have a leading zero"),
            ("-", 1, "expected digits after '-'"),
            ("1.", 2, "expected digits after the decimal point"),
            ("1e+", 3, "expected digits in the exponent"),
            ("tru", 0, "expected a value but found 't'"),
            ("[é]", 1, "expected a value but found 'é'"),
            ("// x\n{}", 0, "expected a value but found '/'"),
        ];
        for (input, offset, message) in cases {
            let e = validate_json(input).unwrap_err();
            assert_eq!(
                (e.offset, e.message.as_str()),
                (*offset, *message),
                "{input:?}"
            );
        }
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(validate_json(&deep).is_ok());
        assert_eq!(minify_json(&deep).unwrap(), deep);
    }

    #[test]
    fn positions_follow_their_tokens() {
        let text = r#"{"a": 1,  "b": "xyz"}"#;
        // Inside "xyz", after the comma, at the start of "b", and at the end.
        let mut positions = vec![7, 8, 10, 17, text.len()];
        let unit = "  ";
        let out = run_json(text, Style::Pretty { unit, base: "" }, &mut positions).unwrap();
        assert_eq!(out, "{\n  \"a\": 1,\n  \"b\": \"xyz\"\n}");
        let at = |p: usize| &out[p..];
        assert!(at(positions[0]).starts_with(",\n"), "after the 1");
        assert!(at(positions[1]).starts_with("\n  \"b\""), "after the comma");
        assert!(at(positions[2]).starts_with("\"b\""));
        assert!(at(positions[3]).starts_with("yz\""));
        assert_eq!(positions[4], out.len());
    }

    #[test]
    fn base64_round_trips_every_padding_length() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
            ("é漢👍", "w6nmvKLwn5GN"),
        ];
        for (plain, encoded) in cases {
            assert_eq!(base64_encode(plain.as_bytes()), encoded);
            assert_eq!(Codec::Base64.decode(encoded).unwrap(), plain);
        }
        // No padding, line breaks, the URL-safe alphabet.
        assert_eq!(base64_decode("Zm9vYg").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9v\nYmFy\n").unwrap(), b"foobar");
        assert_eq!(
            base64_decode("-_-_").unwrap(),
            base64_decode("+/+/").unwrap()
        );
    }

    #[test]
    fn bad_base64_is_refused() {
        assert_eq!(
            base64_decode("Zm9v!").unwrap_err(),
            "'!' is not a Base64 character"
        );
        assert!(base64_decode("Zm9vY").is_err(), "one character too many");
        assert!(base64_decode("Zg==Zg==").is_err(), "text after padding");
        assert!(
            base64_decode("Zm9v=").is_err(),
            "padding without a partial group"
        );
        // 0xFF is not UTF-8.
        assert_eq!(
            Codec::Base64.decode("/w==").unwrap_err(),
            "the decoded bytes are not UTF-8 text"
        );
    }

    #[test]
    fn url_encoding_keeps_only_unreserved_characters() {
        let plain = "a b&c=d/é?~._-+%";
        let encoded = "a%20b%26c%3Dd%2F%C3%A9%3F~._-%2B%25";
        assert_eq!(url_encode(plain.as_bytes()), encoded);
        assert_eq!(Codec::Url.decode(encoded).unwrap(), plain);
        assert_eq!(Codec::Url.decode("a+b%2b%2B").unwrap(), "a+b++");
        assert!(url_decode("100%").is_err());
        assert!(url_decode("%G0").is_err());
        assert_eq!(
            Codec::Url.decode("%FF").unwrap_err(),
            "the decoded bytes are not UTF-8 text"
        );
    }

    #[test]
    fn hashes_use_the_line_ending() {
        let h = hashes("abc", LineEnding::CrLf);
        assert_eq!(h.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!((h.bytes, h.line_breaks), (3, 0));
        let empty = hashes("", LineEnding::Lf);
        assert_eq!(
            empty.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // A line break is hashed as the document's ending.
        let crlf = hashes("a\nb", LineEnding::CrLf);
        assert_eq!((crlf.bytes, crlf.line_breaks), (4, 1));
        assert_eq!(crlf, hashes("a\r\nb", LineEnding::Lf));
    }
}
