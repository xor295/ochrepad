//! Language servers for the open documents (PLAN_completion L1): which
//! server serves a document (found on a thread: the folder it works in and
//! its program on `PATH`), whether the user trusts that folder, starting
//! and restarting servers, keeping their documents in sync, and what they
//! send back (diagnostics, progress, answers to requests).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use ochrepad_core::position::{ColumnUnits, from_lsp_position};
use ochrepad_core::{BufferId, ByteOffset, Document, Editor};
use ochrepad_lsp::config::{self, LspFile, ServerConfig};
use ochrepad_lsp::message::ResponseError;
use ochrepad_lsp::transport::{Launch, Notify, SharedLog};
use ochrepad_lsp::{Client, Event, State, Tag};
use serde_json::Value;

/// A crashed server is started again at most this many times.
const MAX_RESTARTS: u32 = 3;
/// A server with no documents left is stopped after this long.
const IDLE_SECS: f64 = 30.0;

/// Where a document's server was found.
#[derive(Clone, Debug)]
pub struct Found {
    pub server: ServerConfig,
    pub launch: Launch,
    pub root: PathBuf,
}

/// A server waiting for the user's trust in its folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub server: String,
    pub program: String,
    pub root: PathBuf,
}

/// What the user decided about folders and servers (from the settings).
pub struct Trust<'a> {
    pub enabled: bool,
    pub trusted: &'a [PathBuf],
    pub declined: &'a [PathBuf],
    /// Servers never to start.
    pub never: &'a [String],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    /// As the server sent it: (line, column) to (line, column), in its
    /// units.
    pub lsp_range: [(u32, u32); 2],
    /// In the open document, following edits until the server sends new
    /// ones.
    pub range: Option<Range<ByteOffset>>,
}

/// A file's diagnostics.
#[derive(Clone, Debug, Default)]
pub struct FileDiagnostics {
    pub path: Option<PathBuf>,
    pub items: Vec<Diagnostic>,
}

pub struct Session {
    pub name: String,
    pub root: PathBuf,
    pub client: Client,
    pub log: SharedLog,
    found: Found,
    restarts: u32,
    restart_at: Option<f64>,
    idle_since: Option<f64>,
    /// What it is busy with, by progress token.
    pub progress: HashMap<String, String>,
    /// The documents it was given (their buffers).
    buffers: HashSet<BufferId>,
}

impl Session {
    /// A line for the status bar.
    pub fn status(&self) -> String {
        match &self.client.state {
            State::Starting => format!("{}: starting", self.name),
            State::Running => match self.progress.values().next() {
                Some(p) => format!("{}: {p}", self.name),
                None => self.name.clone(),
            },
            State::Stopping => format!("{}: stopping", self.name),
            State::Stopped { reason, .. } => format!("{} stopped: {reason}", self.name),
        }
    }
}

struct DocState {
    session: (String, PathBuf),
    uri: String,
    revision: u64,
}

/// What the app should do or show.
#[derive(Debug, PartialEq)]
pub enum Notice {
    /// A note for the status bar.
    Note(String),
    /// A server asked to show a document (a local file).
    Show { path: PathBuf, line: Option<u32> },
}

type SetupKey = (String, PathBuf);
type SetupAnswer = (SetupKey, Option<Found>);
/// What the setup thread is asked: the key, the file, the open folder and
/// the servers for the file's language.
type SetupRequest = (SetupKey, PathBuf, Option<PathBuf>, Vec<ServerConfig>);

#[derive(Default)]
pub struct LanguageServers {
    servers: Vec<ServerConfig>,
    /// `lsp.json`, and what was wrong with it.
    pub lsp_file: Option<PathBuf>,
    pub lsp_error: Option<String>,
    setup: Option<(Sender<SetupRequest>, Receiver<SetupAnswer>)>,
    /// By (language, the file's folder): the server, or `None` if there is
    /// none.
    found: HashMap<SetupKey, Option<Found>>,
    asked: HashSet<SetupKey>,
    pub sessions: Vec<Session>,
    /// The trust question to show.
    pub question: Option<Question>,
    docs: HashMap<BufferId, DocState>,
    /// By URI.
    pub diagnostics: HashMap<String, FileDiagnostics>,
    /// Answers to requests, by tag, until taken.
    answers: HashMap<Tag, Result<Value, ResponseError>>,
    next_tag: Tag,
    notify: Option<Notify>,
    /// Servers given directly, by language (tests).
    test_servers: HashMap<String, (ServerConfig, Launch)>,
    /// Documents larger than this are not given to servers.
    pub max_bytes: usize,
}

impl LanguageServers {
    /// Reads `lsp.json` (missing is fine).
    pub fn load_config(&mut self, file: PathBuf) {
        let user = match std::fs::read_to_string(&file) {
            Ok(text) => match LspFile::parse(&text) {
                Ok(f) => {
                    self.lsp_error = None;
                    f
                }
                Err(e) => {
                    self.lsp_error = Some(format!("{}: {e}", file.display()));
                    LspFile::default()
                }
            },
            Err(_) => LspFile::default(),
        };
        self.servers = config::servers(&user);
        self.lsp_file = Some(file);
        self.found.clear();
        self.asked.clear();
    }

    /// A server for every file of `language`, started with `launch`
    /// (tests).
    pub fn set_test_server(&mut self, language: &str, server: ServerConfig, launch: Launch) {
        self.test_servers.insert(language.into(), (server, launch));
        self.found.clear();
        self.asked.clear();
    }

    /// Once a frame: finds servers, starts, syncs and stops them, and
    /// takes what they sent.
    pub fn tick(
        &mut self,
        editor: &Editor,
        open_folder: Option<&Path>,
        trust: &Trust<'_>,
        now: f64,
        ctx: &egui::Context,
    ) -> Vec<Notice> {
        let notify = self
            .notify
            .get_or_insert_with(|| {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint())
            })
            .clone();
        let mut notices = Vec::new();
        self.take_setup_answers();
        // A question answered elsewhere (the settings) is not asked.
        if self.question.as_ref().is_some_and(|q| {
            !trust.enabled
                || trust.trusted.contains(&q.root)
                || trust.declined.contains(&q.root)
                || trust.never.contains(&q.server)
        }) {
            self.question = None;
        }
        let mut wanted: HashSet<BufferId> = HashSet::new();
        if trust.enabled {
            for b in editor.buffers() {
                if let Some(path) = b.meta.path.clone()
                    && let Some(language) = b.language.clone()
                    && b.doc().len().0 <= self.max_bytes.max(1)
                    && self.serve(
                        b.id(),
                        &path,
                        &language,
                        b.doc(),
                        b.revision(),
                        open_folder,
                        trust,
                        now,
                        &notify,
                    )
                {
                    wanted.insert(b.id());
                }
            }
        }
        // Documents closed, or no longer served.
        let gone: Vec<BufferId> = self
            .docs
            .keys()
            .filter(|b| !wanted.contains(b))
            .copied()
            .collect();
        for b in gone {
            self.forget(b);
        }
        for s in &mut self.sessions {
            if s.buffers.is_empty() {
                let since = *s.idle_since.get_or_insert(now);
                if now - since > IDLE_SECS || !trust.enabled {
                    s.client.shutdown();
                }
            } else {
                s.idle_since = None;
            }
        }
        self.poll(now, &notify, &mut notices);
        // Stopped as asked: gone.
        self.sessions.retain(|s| {
            !matches!(s.client.state, State::Stopped { asked: true, .. }) || !s.buffers.is_empty()
        });
        notices
    }

    /// Gives `buffer` to its server; whether it has one.
    #[allow(clippy::too_many_arguments)]
    fn serve(
        &mut self,
        buffer: BufferId,
        path: &Path,
        language: &str,
        doc: &Document,
        revision: u64,
        open_folder: Option<&Path>,
        trust: &Trust<'_>,
        now: f64,
        notify: &Notify,
    ) -> bool {
        let Some(dir) = path.parent() else {
            return false;
        };
        let key = (language.to_owned(), dir.to_path_buf());
        let found = match self.found.get(&key) {
            Some(f) => f.clone(),
            None => {
                self.look_for(key, path, open_folder);
                return false;
            }
        };
        let Some(found) = found else {
            return false;
        };
        if trust.never.contains(&found.server.name) || trust.declined.contains(&found.root) {
            return false;
        }
        if !trust.trusted.contains(&found.root) {
            if self.question.is_none() {
                self.question = Some(Question {
                    server: found.server.name.clone(),
                    program: match &found.launch {
                        Launch::Program { program, .. } => program.display().to_string(),
                        Launch::InProcess(_) => found.server.name.clone(),
                    },
                    root: found.root.clone(),
                });
            }
            return false;
        }
        let Some(uri) = ochrepad_lsp::uri::file_uri(path) else {
            return false;
        };
        let session_key = (found.server.name.clone(), found.root.clone());
        // Moved to another server or file (a rename, a language change).
        if self
            .docs
            .get(&buffer)
            .is_some_and(|d| d.session != session_key || d.uri != uri)
        {
            self.forget(buffer);
        }
        let index = match self
            .sessions
            .iter()
            .position(|s| s.name == session_key.0 && s.root == session_key.1)
        {
            Some(i) => i,
            None => {
                self.sessions.push(start_session(found, now, notify));
                self.sessions.len() - 1
            }
        };
        let s = &mut self.sessions[index];
        if let State::Stopped { asked: false, .. } = s.client.state {
            // Waiting to be restarted, or given up.
            return true;
        }
        s.buffers.insert(buffer);
        let language_id = config::language_id(language);
        match self.docs.get_mut(&buffer) {
            Some(d) if d.revision == revision => {}
            Some(d) => {
                d.revision = revision;
                if let Some((old, new)) = s.client.change(&uri, doc)
                    && let Some(f) = self.diagnostics.get_mut(&uri)
                {
                    for item in &mut f.items {
                        if let Some(r) = &mut item.range {
                            *r = map(r.start, &old, &new)..map(r.end, &old, &new);
                        }
                    }
                }
            }
            None => {
                s.client.open(&uri, &language_id, doc);
                self.docs.insert(
                    buffer,
                    DocState {
                        session: session_key,
                        uri: uri.clone(),
                        revision,
                    },
                );
                // Diagnostics the server sent before the document opened.
                let units = s.client.units;
                if let Some(f) = self.diagnostics.get_mut(&uri) {
                    for item in &mut f.items {
                        item.range = Some(offsets(doc, item.lsp_range, units));
                    }
                }
            }
        }
        true
    }

    /// Stops giving `buffer` to its server.
    fn forget(&mut self, buffer: BufferId) {
        let Some(d) = self.docs.remove(&buffer) else {
            return;
        };
        if let Some(s) = self
            .sessions
            .iter_mut()
            .find(|s| s.name == d.session.0 && s.root == d.session.1)
        {
            s.client.close(&d.uri);
            s.buffers.remove(&buffer);
        }
        if let Some(f) = self.diagnostics.get_mut(&d.uri) {
            for item in &mut f.items {
                item.range = None;
            }
        }
    }

    fn look_for(&mut self, key: SetupKey, file: &Path, open_folder: Option<&Path>) {
        if let Some((server, launch)) = self.test_servers.get(&key.0) {
            let root = match open_folder {
                Some(o) if file.starts_with(o) => o.to_path_buf(),
                _ => key.1.clone(),
            };
            let found = Found {
                server: server.clone(),
                launch: launch.clone(),
                root,
            };
            self.found.insert(key, Some(found));
            return;
        }
        // Programs on PATH only once the configuration is known: not in
        // tests or benches, which have no storage.
        if self.lsp_file.is_none() {
            self.found.insert(key, None);
            return;
        }
        if !self.asked.insert(key.clone()) {
            return;
        }
        let servers: Vec<ServerConfig> = self
            .servers
            .iter()
            .filter(|s| s.languages.contains(&key.0))
            .cloned()
            .collect();
        if servers.is_empty() {
            self.found.insert(key, None);
            return;
        }
        if self.setup.is_none() {
            let (tx, rx) = channel::<SetupRequest>();
            let (answer_tx, answers) = channel();
            let notify = self.notify.clone();
            let spawned = std::thread::Builder::new()
                .name("ochrepad-lsp-setup".into())
                .spawn(move || {
                    while let Ok((key, file, open_folder, servers)) = rx.recv() {
                        let found = setup(&file, open_folder.as_deref(), &servers);
                        if answer_tx.send((key, found)).is_err() {
                            return;
                        }
                        if let Some(n) = &notify {
                            n();
                        }
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.setup = Some((tx, answers));
        }
        if let Some((tx, _)) = &self.setup {
            let _ = tx.send((
                key,
                file.to_path_buf(),
                open_folder.map(Path::to_path_buf),
                servers,
            ));
        }
    }

    fn take_setup_answers(&mut self) {
        let Some((_, rx)) = &self.setup else {
            return;
        };
        while let Ok((key, found)) = rx.try_recv() {
            self.found.insert(key, found);
        }
    }

    fn poll(&mut self, now: f64, notify: &Notify, notices: &mut Vec<Notice>) {
        for i in 0..self.sessions.len() {
            let events = self.sessions[i].client.poll();
            for e in events {
                self.event(i, e, now, notices);
            }
            let s = &mut self.sessions[i];
            if s.restart_at.is_some_and(|t| now >= t) {
                s.restart_at = None;
                s.restarts += 1;
                s.progress.clear();
                s.client = Client::start(
                    &s.found.server,
                    &s.found.launch,
                    &s.root,
                    s.log.clone(),
                    notify.clone(),
                );
                // Its documents are opened again by the next tick.
                let buffers: Vec<BufferId> = s.buffers.drain().collect();
                for b in buffers {
                    self.docs.remove(&b);
                }
            }
        }
    }

    fn event(&mut self, i: usize, e: Event, now: f64, notices: &mut Vec<Notice>) {
        let s = &mut self.sessions[i];
        match e {
            Event::Started => {}
            Event::Answer { tag, result } => {
                self.answers.insert(tag, result);
            }
            Event::Diagnostics {
                uri, diagnostics, ..
            } => {
                let units = s.client.units;
                let text = s.client.text(&uri).cloned();
                let items = diagnostics
                    .iter()
                    .filter_map(|d| diagnostic(d, text.as_ref(), units))
                    .collect();
                self.diagnostics.insert(
                    uri.clone(),
                    FileDiagnostics {
                        path: ochrepad_lsp::uri::file_path(&uri),
                        items,
                    },
                );
            }
            Event::Message { kind, text } => {
                // Errors and warnings only; the rest is in the log.
                if kind <= 2 {
                    notices.push(Notice::Note(format!("{}: {text}", s.name)));
                }
            }
            Event::ShowDocument { uri, selection } => {
                if let Some(path) = ochrepad_lsp::uri::file_path(&uri) {
                    let line = selection
                        .as_ref()
                        .and_then(|r| r.get("start")?.get("line")?.as_u64())
                        .map(|l| l as u32);
                    notices.push(Notice::Show { path, line });
                }
            }
            Event::ApplyEdit { id, .. } => {
                // Only during the user's own rename or code action (L3).
                s.client.answer_apply_edit(
                    id,
                    false,
                    Some("ochrepad applies edits only for its own renames and code actions"),
                );
            }
            Event::Progress { token, text } => match text {
                Some(t) => {
                    s.progress.insert(token, t);
                }
                None => {
                    s.progress.remove(&token);
                }
            },
            Event::Stopped { reason, asked } => {
                s.progress.clear();
                // Its diagnostics are no longer current.
                let root = s.root.clone();
                self.diagnostics
                    .retain(|_, f| f.path.as_ref().is_none_or(|p| !p.starts_with(&root)));
                let s = &mut self.sessions[i];
                if !asked {
                    if s.restarts < MAX_RESTARTS {
                        s.restart_at = Some(now + 1.0 + s.restarts as f64);
                    } else {
                        notices.push(Notice::Note(format!(
                            "{} stopped: {reason} (View > Language Servers can restart it)",
                            s.name
                        )));
                    }
                }
            }
        }
    }

    /// Starts a session again (View > Language Servers).
    pub fn restart(&mut self, index: usize) {
        let notify = self.notify.clone().unwrap_or_else(|| Arc::new(|| {}));
        let Some(s) = self.sessions.get_mut(index) else {
            return;
        };
        s.client.shutdown();
        s.restarts = 0;
        s.restart_at = None;
        s.progress.clear();
        s.client = Client::start(
            &s.found.server,
            &s.found.launch,
            &s.root,
            s.log.clone(),
            notify,
        );
        // Its documents are opened again by the next tick.
        let buffers: Vec<BufferId> = s.buffers.drain().collect();
        for b in buffers {
            self.docs.remove(&b);
        }
    }

    /// Stops a session; it is not restarted until its documents are opened
    /// again or Restart is chosen.
    pub fn stop(&mut self, index: usize) {
        if let Some(s) = self.sessions.get_mut(index) {
            s.restarts = MAX_RESTARTS;
            s.restart_at = None;
            s.client.shutdown();
        }
    }

    /// Stops every server (the app is closing, or servers were turned off).
    pub fn stop_all(&mut self) {
        for s in &mut self.sessions {
            s.client.shutdown();
        }
    }

    /// The session serving `buffer`.
    pub fn session_of(&self, buffer: BufferId) -> Option<&Session> {
        let d = self.docs.get(&buffer)?;
        self.sessions
            .iter()
            .find(|s| s.name == d.session.0 && s.root == d.session.1)
    }

    fn session_of_mut(&mut self, buffer: BufferId) -> Option<(&mut Session, &str)> {
        let d = self.docs.get(&buffer)?;
        let s = self
            .sessions
            .iter_mut()
            .find(|s| s.name == d.session.0 && s.root == d.session.1)?;
        Some((s, d.uri.as_str()))
    }

    /// The offsets a server's range names in `doc`, `buffer`'s document.
    pub fn range_in(
        &self,
        buffer: BufferId,
        doc: &Document,
        range: &Value,
    ) -> Option<Range<ByteOffset>> {
        self.session_of(buffer)?.client.range(doc, range)
    }

    /// The diagnostics of `buffer`'s document.
    pub fn diagnostics_of(&self, buffer: BufferId) -> &[Diagnostic] {
        self.docs
            .get(&buffer)
            .and_then(|d| self.diagnostics.get(&d.uri))
            .map_or(&[], |f| f.items.as_slice())
    }

    /// The document was saved.
    pub fn saved(&mut self, buffer: BufferId) {
        if let Some((s, uri)) = self.session_of_mut(buffer) {
            let uri = uri.to_owned();
            s.client.save(&uri);
        }
    }

    /// Sends a request about `buffer`'s document (after bringing the
    /// server's copy up to date with `doc`); its answer comes back from
    /// [`Self::take_answer`]. `None` when no running server serves it or
    /// the server cannot do `capability`.
    pub fn request(
        &mut self,
        buffer: BufferId,
        doc: &Document,
        capability: &str,
        method: &str,
        params: impl FnOnce(&Client, &str) -> Value,
        timeout: Duration,
    ) -> Option<(Tag, i64)> {
        self.next_tag += 1;
        let tag = self.next_tag;
        let (s, uri) = self.session_of_mut(buffer)?;
        if !s.client.is_running() || !s.client.can(capability) {
            return None;
        }
        let uri = uri.to_owned();
        s.client.change(&uri, doc);
        let p = params(&s.client, &uri);
        let id = s.client.request(method, p, tag, timeout)?;
        Some((tag, id))
    }

    /// Gives up on a request about `buffer`'s document.
    pub fn cancel(&mut self, buffer: BufferId, id: i64) {
        if let Some((s, _)) = self.session_of_mut(buffer) {
            s.client.cancel(id);
        }
    }

    /// A request's answer, once it came.
    pub fn take_answer(&mut self, tag: Tag) -> Option<Result<Value, ResponseError>> {
        self.answers.remove(&tag)
    }
}

/// The editor's underlines for `diagnostics` (those in the open document),
/// sorted by start.
pub fn underlines(
    diagnostics: &[Diagnostic],
    visuals: &egui::Visuals,
) -> Vec<crate::editor_view::Underline> {
    let mut out: Vec<crate::editor_view::Underline> = diagnostics
        .iter()
        .filter_map(|d| {
            let rank = d.severity as u8;
            Some(crate::editor_view::Underline {
                range: d.range.clone()?,
                color: crate::theme::diagnostic_color(visuals, rank),
                dotted: d.severity == Severity::Hint,
                rank,
            })
        })
        .collect();
    out.sort_by_key(|u| (u.range.start, u.rank));
    out
}

fn start_session(found: Found, now: f64, notify: &Notify) -> Session {
    let log = SharedLog::default();
    let client = Client::start(
        &found.server,
        &found.launch,
        &found.root,
        log.clone(),
        notify.clone(),
    );
    Session {
        name: found.server.name.clone(),
        root: found.root.clone(),
        client,
        log,
        found,
        restarts: 0,
        restart_at: None,
        idle_since: Some(now),
        progress: HashMap::new(),
        buffers: HashSet::new(),
    }
}

/// On the setup thread: the folder the server works in and the first of
/// `servers` whose program is found.
fn setup(file: &Path, open_folder: Option<&Path>, servers: &[ServerConfig]) -> Option<Found> {
    let root = config::find_root(file, open_folder)?;
    let mut avoid: Vec<&Path> = vec![&root];
    if let Some(o) = open_folder {
        avoid.push(o);
    }
    for server in servers {
        let Some((program, args)) = server.command.split_first() else {
            continue;
        };
        if let Some(program) = config::resolve_program(program, &avoid) {
            return Some(Found {
                server: server.clone(),
                launch: Launch::Program {
                    program,
                    args: args.to_vec(),
                    cwd: root.clone(),
                },
                root,
            });
        }
    }
    None
}

/// `pos` after the edit that replaced `old` with `new` (inside the
/// replaced text: the end of the new text).
fn map(pos: ByteOffset, old: &Range<ByteOffset>, new: &Range<ByteOffset>) -> ByteOffset {
    if pos <= old.start {
        pos
    } else if pos >= old.end {
        ByteOffset(pos.0 - old.end.0 + new.end.0)
    } else {
        new.end
    }
}

fn offsets(doc: &Document, r: [(u32, u32); 2], units: ColumnUnits) -> Range<ByteOffset> {
    let a = from_lsp_position(doc, r[0].0, r[0].1, units);
    let b = from_lsp_position(doc, r[1].0, r[1].1, units);
    a.min(b)..a.max(b)
}

/// A diagnostic as the server sent it (`None` if it is not one).
fn diagnostic(v: &Value, text: Option<&Document>, units: ColumnUnits) -> Option<Diagnostic> {
    let pos = |p: &Value| -> Option<(u32, u32)> {
        let n = |k: &str| {
            p.get(k)?
                .as_u64()
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        };
        Some((n("line")?, n("character")?))
    };
    let range = v.get("range")?;
    let lsp_range = [pos(range.get("start")?)?, pos(range.get("end")?)?];
    let severity = match v.get("severity").and_then(Value::as_i64) {
        Some(2) => Severity::Warning,
        Some(3) => Severity::Information,
        Some(4) => Severity::Hint,
        _ => Severity::Error,
    };
    let code = match v.get("code") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    Some(Diagnostic {
        severity,
        message: v.get("message")?.as_str()?.to_owned(),
        source: v.get("source").and_then(Value::as_str).map(str::to_owned),
        code,
        lsp_range,
        range: text.map(|t| offsets(t, lsp_range, units)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_follow_an_edit() {
        let old = ByteOffset(10)..ByteOffset(14);
        let new = ByteOffset(10)..ByteOffset(11);
        assert_eq!(map(ByteOffset(3), &old, &new), ByteOffset(3));
        assert_eq!(map(ByteOffset(12), &old, &new), ByteOffset(11));
        assert_eq!(map(ByteOffset(20), &old, &new), ByteOffset(17));
    }

    #[test]
    fn diagnostics_from_json() {
        let doc = Document::new("let x = 1;\n");
        let d = diagnostic(
            &serde_json::json!({
                "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 5}},
                "severity": 2, "message": "unused", "source": "rustc", "code": 1
            }),
            Some(&doc),
            ColumnUnits::Utf16,
        )
        .unwrap();
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.range, Some(ByteOffset(4)..ByteOffset(5)));
        assert_eq!(d.code.as_deref(), Some("1"));
        assert!(
            diagnostic(
                &serde_json::json!({"message": "no range"}),
                None,
                ColumnUnits::Utf16
            )
            .is_none()
        );
    }
}
