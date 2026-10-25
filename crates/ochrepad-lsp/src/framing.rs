//! The base protocol's framing: a `Content-Length` header, a blank line,
//! then that many bytes of JSON. Anything else is an error that ends the
//! session (the caller's choice), never a panic.

use std::io::{self, BufRead, Read, Write};

/// The largest message accepted.
pub const MAX_MESSAGE: usize = 64 << 20;
/// Header lines are short; a longer one is not a header.
const MAX_HEADER_LINE: usize = 1024;
const MAX_HEADERS: usize = 16;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Reads one message's body. `Ok(None)` at the end of the stream before a
/// message starts.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut line = Vec::new();
    for n in 0.. {
        if n > MAX_HEADERS {
            return Err(invalid("too many header lines"));
        }
        line.clear();
        let read = Read::take(&mut *r, MAX_HEADER_LINE as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return if n == 0 {
                Ok(None)
            } else {
                Err(invalid("the stream ended in a header"))
            };
        }
        if line.last() != Some(&b'\n') {
            return Err(invalid("a header line is too long"));
        }
        let text = std::str::from_utf8(&line).map_err(|_| invalid("a header is not text"))?;
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        let Some((name, value)) = text.split_once(':') else {
            return Err(invalid(format!("not a header: {text:?}")));
        };
        if name.trim().eq_ignore_ascii_case("Content-Length") {
            let n: usize = value
                .trim()
                .parse()
                .map_err(|_| invalid(format!("a bad Content-Length: {value:?}")))?;
            if n > MAX_MESSAGE {
                return Err(invalid(format!("a message of {n} bytes is too large")));
            }
            length = Some(n);
        }
    }
    let length = length.ok_or_else(|| invalid("no Content-Length"))?;
    let mut body = vec![0; length];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Writes one message.
pub fn write_message(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8]) -> io::Result<Option<Vec<u8>>> {
        read_message(&mut io::BufReader::new(bytes))
    }

    #[test]
    fn messages_round_trip() {
        let mut out = Vec::new();
        write_message(&mut out, b"{\"a\":1}").unwrap();
        write_message(&mut out, "{\"é\":2}".as_bytes()).unwrap();
        let mut r = io::BufReader::new(&out[..]);
        assert_eq!(read_message(&mut r).unwrap().unwrap(), b"{\"a\":1}");
        assert_eq!(
            read_message(&mut r).unwrap().unwrap(),
            "{\"é\":2}".as_bytes()
        );
        assert!(read_message(&mut r).unwrap().is_none(), "the end");
    }

    #[test]
    fn other_headers_and_bare_newlines_are_accepted() {
        let msg =
            b"Content-Type: application/vscode-jsonrpc; charset=utf-8\ncontent-length: 2\n\n{}";
        assert_eq!(read(msg).unwrap().unwrap(), b"{}");
    }

    #[test]
    fn bad_input_is_an_error() {
        for bad in [
            &b"Content-Length: 5\r\n\r\n{}"[..],
            b"Content-Length: x\r\n\r\n",
            b"Content-Length: 99999999999\r\n\r\n",
            b"garbage\r\n\r\n",
            b"\r\n{}",
            b"Content-Length: 2\r\n",
        ] {
            assert!(read(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let long = format!("X-Long: {}\r\n", "a".repeat(5000));
        assert!(read(long.as_bytes()).is_err());
    }
}
