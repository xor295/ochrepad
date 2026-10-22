//! One ochrepad per configuration folder: a later launch hands its files to
//! the running instance and exits, as Notepad++ does, so two windows never
//! write the same session.
//!
//! The first instance holds an exclusive lock on `instance.lock` for as long
//! as it runs (the OS drops it when the process ends, even after a crash),
//! listens on a loopback port, and writes the port and a random token to
//! `instance.json` (`0600` on Linux). A later launch that cannot take the
//! lock connects to that port, sends the token and its paths (made absolute
//! against its own working folder), waits for the reply and exits. The token
//! keeps other users of the machine from sending paths.

use std::fs::{File, OpenOptions, TryLockError};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const LOCK_FILE: &str = "instance.lock";
const INFO_FILE: &str = "instance.json";
/// Sent first, so a stray connection is told apart from a launch.
const MAGIC: &[u8; 8] = b"ochrpd01";
/// The running instance's answer once the paths are queued.
const ACK: u8 = b'1';
/// How long a launch keeps trying while another one is starting or closing.
const START_WAIT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PATHS: u32 = 10_000;
const MAX_ITEM: u32 = 64 * 1024;

/// What a launch should do.
pub enum Start {
    /// This is the first instance; it receives later launches' paths.
    Primary(Primary),
    /// The paths went to the running instance; this process should exit.
    Forwarded,
}

/// A later launch: its paths (empty when it was started without any; the
/// window should come to the front either way).
#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub paths: Vec<PathBuf>,
}

/// The first instance's side: keeps the lock and receives launches.
pub struct Primary {
    _lock: File,
    launches: Receiver<Launch>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl Primary {
    /// Lets the listener wake the UI when a launch arrives.
    pub fn set_context(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// The launches that arrived since the last call.
    pub fn take(&self) -> Vec<Launch> {
        self.launches.try_iter().collect()
    }
}

#[derive(Serialize, Deserialize)]
struct Info {
    port: u16,
    token: String,
}

/// Becomes the first instance in `dir`, or forwards `paths` to the one that
/// is running. An error means neither worked (no file locks, no loopback);
/// the caller then runs on its own, as before single instance.
pub fn start(dir: &Path, paths: &[PathBuf]) -> io::Result<Start> {
    std::fs::create_dir_all(dir)?;
    let paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.clone()))
        .collect();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE))?;
    let deadline = Instant::now() + START_WAIT;
    loop {
        match lock.try_lock() {
            Ok(()) => return listen(dir, lock).map(Start::Primary),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => return Err(e),
        }
        // The running instance may still be writing its port, or closing.
        let error = match forward(dir, &paths) {
            Ok(()) => return Ok(Start::Forwarded),
            Err(e) => e,
        };
        if Instant::now() >= deadline {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn listen(dir: &Path, lock: File) -> io::Result<Primary> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let info = Info {
        port: listener.local_addr()?.port(),
        token: new_token(),
    };
    let json = serde_json::to_vec(&info).map_err(io::Error::other)?;
    ochrepad_fileio::write_atomic(&dir.join(INFO_FILE), &json)?;
    let (send, launches) = channel();
    let ctx = Arc::new(OnceLock::new());
    let woken = Arc::clone(&ctx);
    std::thread::Builder::new()
        .name("ochrepad-instance".into())
        .spawn(move || serve(&listener, &info.token, &send, &woken))?;
    Ok(Primary {
        _lock: lock,
        launches,
        ctx,
    })
}

fn serve(
    listener: &TcpListener,
    token: &str,
    send: &Sender<Launch>,
    ctx: &OnceLock<egui::Context>,
) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        // A bad or slow connection is dropped; the next one is served.
        let Ok(launch) = receive(&mut stream, token) else {
            continue;
        };
        if send.send(launch).is_err() {
            return;
        }
        if let Some(ctx) = ctx.get() {
            ctx.request_repaint();
        }
        let _ = stream.write_all(&[ACK]);
    }
}

fn receive(stream: &mut TcpStream, token: &str) -> io::Result<Launch> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut magic = [0; 8];
    stream.read_exact(&mut magic)?;
    if &magic != MAGIC || read_item(stream)? != token.as_bytes() {
        return Err(io::Error::other("not an ochrepad launch"));
    }
    let count = read_u32(stream)?;
    if count > MAX_PATHS {
        return Err(io::Error::other("too many paths"));
    }
    let paths = (0..count)
        .map(|_| read_item(stream).map(|b| path_from_bytes(&b)))
        .collect::<io::Result<_>>()?;
    Ok(Launch { paths })
}

fn forward(dir: &Path, paths: &[PathBuf]) -> io::Result<()> {
    let info: Info = ochrepad_fileio::load_json(&dir.join(INFO_FILE))
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
    let mut stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut message = MAGIC.to_vec();
    write_item(&mut message, info.token.as_bytes());
    let count = u32::try_from(paths.len()).map_err(io::Error::other)?;
    message.extend_from_slice(&count.to_le_bytes());
    for p in paths {
        write_item(&mut message, &path_to_bytes(p));
    }
    stream.write_all(&message)?;
    let mut ack = [0];
    stream.read_exact(&mut ack)?;
    if ack[0] == ACK {
        Ok(())
    } else {
        Err(io::Error::other("unexpected reply"))
    }
}

fn write_item(out: &mut Vec<u8>, bytes: &[u8]) {
    // Paths and tokens are far below 4 GB.
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn read_u32(stream: &mut impl Read) -> io::Result<u32> {
    let mut n = [0; 4];
    stream.read_exact(&mut n)?;
    Ok(u32::from_le_bytes(n))
}

fn read_item(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let len = read_u32(stream)?;
    if len > MAX_ITEM {
        return Err(io::Error::other("item too long"));
    }
    let mut bytes = vec![0; len as usize];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// A path's bytes as the OS has them, so names that are not valid Unicode
/// arrive intact: raw bytes on Linux, UTF-16 code units on Windows.
#[cfg(unix)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(windows)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    let wide: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect();
    PathBuf::from(std::ffi::OsString::from_wide(&wide))
}

/// 128 random bits as hex, from the OS-seeded keys of `RandomState`.
fn new_token() -> String {
    let half = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        h.write_u32(std::process::id());
        h.finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for_launch(primary: &Primary) -> Vec<Launch> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let got = primary.take();
            if !got.is_empty() || Instant::now() > deadline {
                return got;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn later_launches_reach_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let Start::Primary(primary) = start(dir.path(), &[]).unwrap() else {
            panic!("the first launch should be the primary");
        };
        let a = dir.path().join("a.txt");
        let b = dir.path().join("ünïcode b.md");
        assert!(matches!(
            start(dir.path(), &[a.clone(), b.clone()]).unwrap(),
            Start::Forwarded
        ));
        assert_eq!(
            wait_for_launch(&primary),
            vec![Launch { paths: vec![a, b] }]
        );
        // No paths: the running window just comes to the front.
        assert!(matches!(start(dir.path(), &[]).unwrap(), Start::Forwarded));
        assert_eq!(wait_for_launch(&primary), vec![Launch { paths: vec![] }]);

        // Once it exits, the next launch takes over.
        drop(primary);
        assert!(matches!(start(dir.path(), &[]).unwrap(), Start::Primary(_)));
    }

    #[test]
    fn relative_paths_are_made_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let Start::Primary(primary) = start(dir.path(), &[]).unwrap() else {
            panic!("the first launch should be the primary");
        };
        start(dir.path(), &[PathBuf::from("notes.txt")]).unwrap();
        let got = wait_for_launch(&primary);
        let expected = std::env::current_dir().unwrap().join("notes.txt");
        assert_eq!(
            got,
            vec![Launch {
                paths: vec![expected]
            }]
        );
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let Start::Primary(primary) = start(dir.path(), &[]).unwrap() else {
            panic!("the first launch should be the primary");
        };
        let info: Info = ochrepad_fileio::load_json(&dir.path().join(INFO_FILE))
            .unwrap()
            .unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, info.port)).unwrap();
        let mut message = MAGIC.to_vec();
        write_item(&mut message, b"guess");
        message.extend_from_slice(&1u32.to_le_bytes());
        write_item(&mut message, b"/etc/passwd");
        stream.write_all(&message).unwrap();
        let mut reply = Vec::new();
        let _ = stream.read_to_end(&mut reply);
        assert!(reply.is_empty());
        assert!(primary.take().is_empty());
    }
}
