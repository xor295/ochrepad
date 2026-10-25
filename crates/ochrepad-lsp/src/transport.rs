//! A server's process (or, in tests, a thread) and the threads that carry
//! its messages: one writes, one reads, one keeps its stderr in a log.
//! Nothing here blocks the caller.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::framing;
use crate::message::Message;

/// Called after each message arrives (the app repaints).
pub type Notify = Arc<dyn Fn() + Send + Sync>;
/// A server run on a thread, given its stdin and stdout (tests).
pub type InProcess = Arc<dyn Fn(std::io::PipeReader, std::io::PipeWriter) + Send + Sync>;

/// How to start a server.
#[derive(Clone)]
pub enum Launch {
    Program {
        /// An absolute path (see `config::resolve_program`).
        program: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
    },
    InProcess(InProcess),
}

impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Launch::Program { program, args, .. } => {
                write!(f, "{} {}", program.display(), args.join(" "))
            }
            Launch::InProcess(_) => write!(f, "(in process)"),
        }
    }
}

/// What the reader thread passes on.
#[derive(Debug)]
pub enum Incoming {
    Message(Message),
    /// The stream ended, or its framing broke (the session is over).
    Closed(String),
}

/// Lines kept from a server's stderr and about its session.
const LOG_LINES: usize = 5000;
const LOG_LINE: usize = 2000;

#[derive(Default)]
pub struct Log {
    lines: VecDeque<String>,
}

impl Log {
    pub fn push(&mut self, line: &str) {
        let mut line = line.trim_end().to_owned();
        if line.len() > LOG_LINE {
            let mut end = LOG_LINE;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            line.truncate(end);
            line.push('…');
        }
        if self.lines.len() == LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }
}

pub type SharedLog = Arc<Mutex<Log>>;

pub fn log(log: &SharedLog, line: &str) {
    if let Ok(mut l) = log.lock() {
        l.push(line);
    }
}

pub struct Transport {
    outgoing: Option<Sender<Vec<u8>>>,
    pub incoming: Receiver<Incoming>,
    child: Option<Child>,
}

/// How long a server has to exit by itself before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(2);

impl Transport {
    pub fn start(launch: &Launch, log: SharedLog, notify: Notify) -> std::io::Result<Self> {
        let (out_tx, out_rx) = channel::<Vec<u8>>();
        let (in_tx, incoming) = channel();
        let (reader, writer, child): (Box<dyn Read + Send>, Box<dyn Write + Send>, _) = match launch
        {
            Launch::Program { program, args, cwd } => {
                let mut command = Command::new(program);
                command
                    .args(args)
                    .current_dir(cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                #[cfg(windows)]
                {
                    // No console window for a console program (ochrepad is
                    // a GUI program in release builds).
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    command.creation_flags(CREATE_NO_WINDOW);
                }
                let mut child = command.spawn()?;
                let (Some(stdin), Some(stdout), Some(stderr)) =
                    (child.stdin.take(), child.stdout.take(), child.stderr.take())
                else {
                    let _ = child.kill();
                    return Err(std::io::Error::other("no pipes"));
                };
                let err_log = log.clone();
                spawn("ochrepad-lsp-stderr", move || {
                    let mut r = BufReader::new(stderr);
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        match r.by_ref().take(64 * 1024).read_until(b'\n', &mut line) {
                            Ok(0) | Err(_) => return,
                            Ok(_) => {
                                crate::transport::log(&err_log, &String::from_utf8_lossy(&line))
                            }
                        }
                    }
                })?;
                (Box::new(stdout), Box::new(stdin), Some(child))
            }
            Launch::InProcess(run) => {
                let (server_in, to_server) = std::io::pipe()?;
                let (from_server, server_out) = std::io::pipe()?;
                let run = run.clone();
                spawn("ochrepad-lsp-fake", move || run(server_in, server_out))?;
                (Box::new(from_server), Box::new(to_server), None)
            }
        };
        spawn("ochrepad-lsp-write", move || {
            let mut w = std::io::BufWriter::new(writer);
            // Ends when the sender is dropped: the server's stdin closes.
            while let Ok(body) = out_rx.recv() {
                if framing::write_message(&mut w, &body).is_err() {
                    return;
                }
            }
        })?;
        spawn("ochrepad-lsp-read", move || {
            let mut r = BufReader::new(reader);
            loop {
                let incoming = match framing::read_message(&mut r) {
                    Ok(Some(body)) => match Message::parse(&body) {
                        Ok(m) => Incoming::Message(m),
                        Err(e) => {
                            crate::transport::log(
                                &log,
                                &format!("[ochrepad] a message was dropped: {e}"),
                            );
                            continue;
                        }
                    },
                    Ok(None) => Incoming::Closed("the server closed its output".into()),
                    Err(e) => Incoming::Closed(format!("the server's output broke off: {e}")),
                };
                let closed = matches!(incoming, Incoming::Closed(_));
                if in_tx.send(incoming).is_err() {
                    return;
                }
                notify();
                if closed {
                    return;
                }
            }
        })?;
        Ok(Self {
            outgoing: Some(out_tx),
            incoming,
            child,
        })
    }

    /// Queues a message for the server.
    pub fn send(&self, m: &Message) {
        if let Some(out) = &self.outgoing {
            let _ = out.send(m.to_bytes());
        }
    }

    /// Closes the server's input; a process that has not exited after a
    /// moment is killed (on a thread).
    pub fn close(&mut self) {
        self.outgoing = None;
        if let Some(mut child) = self.child.take() {
            let _ = std::thread::Builder::new()
                .name("ochrepad-lsp-exit".into())
                .spawn(move || {
                    let start = Instant::now();
                    while start.elapsed() < EXIT_GRACE {
                        if !matches!(child.try_wait(), Ok(None)) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                });
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.close();
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(|_| ())
}
