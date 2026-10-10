//! Macro recording, playback and saved macros (PLAN M9). Recording itself
//! lives in the core `Editor`; this keeps the last recording, the saved
//! macros (JSON in the config directory) and the dialogs.

use std::path::PathBuf;

use egui::{Id, Key, KeyboardShortcut, Modal, Modifiers, Ui};
use ochrepad_core::{Action, Editor, MacroError, NamedMacro, Repeat, ViewId};
use ochrepad_fileio as fileio;

/// Which macro to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacroSource {
    /// The last recording.
    Recorded,
    /// A saved macro by index.
    Saved(usize),
}

enum Dialog {
    Save {
        name: String,
        shortcut: String,
        error: Option<String>,
    },
    Run {
        source: MacroSource,
        times: usize,
        until_end: bool,
    },
}

#[derive(Default)]
pub struct Macros {
    /// The last recorded macro.
    pub recorded: Vec<Action>,
    pub saved: Vec<NamedMacro>,
    file: Option<PathBuf>,
    dialog: Option<Dialog>,
}

/// Parses "Ctrl+Shift+F5"-style text. Ctrl and Cmd both mean the platform
/// command key.
pub fn parse_shortcut(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => modifiers = modifiers.plus(Modifiers::COMMAND),
            "alt" => modifiers = modifiers.plus(Modifiers::ALT),
            "shift" => modifiers = modifiers.plus(Modifiers::SHIFT),
            _ => return None,
        }
    }
    let key = Key::from_name(key).or_else(|| Key::from_name(&key.to_ascii_uppercase()))?;
    Some(KeyboardShortcut::new(modifiers, key))
}

/// A short description of an action for messages.
pub fn describe(action: &Action) -> String {
    let short = |s: &str| {
        let s: String = s.chars().take(30).collect();
        format!("{s:?}")
    };
    match action {
        Action::InsertText(t) => format!("type {}", short(t)),
        Action::Paste(t) | Action::PasteBlock(t) => format!("paste {}", short(t)),
        Action::Find { query, backward } => format!(
            "find {} {}",
            if *backward { "previous" } else { "next" },
            short(&query.pattern)
        ),
        Action::Replace { query, .. } => format!("replace {}", short(&query.pattern)),
        Action::ReplaceAll { query, .. } => format!("replace all {}", short(&query.pattern)),
        Action::SelectAllMatches(query) => format!("select all {}", short(&query.pattern)),
        Action::MoveCursor { motion, extend } => {
            format!("{} {motion:?}", if *extend { "extend" } else { "move" })
        }
        other => format!("{other:?}"),
    }
}

impl Macros {
    /// Uses `path` for saved macros, loading what is there.
    pub fn set_file(&mut self, path: PathBuf) -> Result<(), String> {
        let loaded = fileio::load_json::<Vec<NamedMacro>>(&path);
        self.file = Some(path);
        self.saved = loaded?.unwrap_or_default();
        Ok(())
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.file else {
            return Err("there is no configuration directory to save macros in".to_owned());
        };
        fileio::save_json(path, &self.saved)
            .map_err(|e| format!("could not save macros to {}: {e}", path.display()))
    }

    /// Starts recording, or stops and keeps the recording.
    pub fn toggle_recording(&mut self, editor: &mut Editor) {
        if editor.is_recording() {
            self.recorded = editor.stop_recording();
        } else {
            editor.start_recording();
        }
    }

    /// Saves the last recording as `name`, replacing a macro of that name.
    pub fn save_recorded(&mut self, name: &str, shortcut: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Give the macro a name.".to_owned());
        }
        if self.recorded.is_empty() {
            return Err("Nothing has been recorded.".to_owned());
        }
        let shortcut = shortcut.trim();
        if !shortcut.is_empty() && parse_shortcut(shortcut).is_none() {
            return Err(format!(
                "\"{shortcut}\" is not a shortcut, e.g. Ctrl+Shift+1."
            ));
        }
        let m = NamedMacro {
            name: name.to_owned(),
            shortcut: (!shortcut.is_empty()).then(|| shortcut.to_owned()),
            actions: self.recorded.clone(),
        };
        match self.saved.iter_mut().find(|s| s.name == m.name) {
            Some(existing) => *existing = m,
            None => self.saved.push(m),
        }
        self.persist()
    }

    pub fn delete(&mut self, index: usize) -> Result<(), String> {
        if index < self.saved.len() {
            self.saved.remove(index);
        }
        self.persist()
    }

    fn actions(&self, source: MacroSource) -> Option<(&str, &[Action])> {
        match source {
            MacroSource::Recorded => Some(("recorded macro", &self.recorded)),
            MacroSource::Saved(i) => self.saved.get(i).map(|m| (m.name.as_str(), &m.actions[..])),
        }
    }

    /// Plays a macro; returns a message to show on failure.
    pub fn play(
        &self,
        editor: &mut Editor,
        view: ViewId,
        source: MacroSource,
        repeat: Repeat,
    ) -> Result<usize, String> {
        let Some((name, actions)) = self.actions(source) else {
            return Err("No such macro.".to_owned());
        };
        if editor.is_recording() {
            return Err("Stop recording before playing a macro.".to_owned());
        }
        editor.play_macro(view, actions, repeat).map_err(
            |MacroError {
                 run,
                 step,
                 action,
                 error,
             }| {
                format!(
                    "The {name} stopped at step {step} ({}) of run {run}: {error}.",
                    describe(&action)
                )
            },
        )
    }

    /// The saved macro bound to a shortcut pressed this frame, if any.
    pub fn shortcut_pressed(&self, ui: &mut Ui) -> Option<MacroSource> {
        self.saved.iter().enumerate().find_map(|(i, m)| {
            let s = parse_shortcut(m.shortcut.as_deref()?)?;
            ui.input_mut(|inp| inp.consume_shortcut(&s))
                .then_some(MacroSource::Saved(i))
        })
    }

    pub fn open_save_dialog(&mut self) {
        self.dialog = Some(Dialog::Save {
            name: String::new(),
            shortcut: String::new(),
            error: None,
        });
    }

    pub fn open_run_dialog(&mut self) {
        self.dialog = Some(Dialog::Run {
            source: if self.recorded.is_empty() && !self.saved.is_empty() {
                MacroSource::Saved(0)
            } else {
                MacroSource::Recorded
            },
            times: 1,
            until_end: false,
        });
    }

    /// Draws an open dialog. Returns a playback request from the run dialog.
    pub fn dialogs(&mut self, ui: &mut Ui) -> Option<(MacroSource, Repeat)> {
        let mut close = false;
        let mut run = None;
        match &mut self.dialog {
            None => return None,
            Some(Dialog::Save {
                name,
                shortcut,
                error,
            }) => {
                let resp = Modal::new(Id::new("ochrepad-save-macro")).show(ui.ctx(), |ui| {
                    ui.heading("Save recorded macro");
                    egui::Grid::new("save-macro-grid").show(ui, |ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(name);
                        ui.end_row();
                        ui.label("Shortcut (optional):");
                        ui.add(egui::TextEdit::singleline(shortcut).hint_text("Ctrl+Shift+1"));
                        ui.end_row();
                    });
                    if let Some(e) = error {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    ui.horizontal(|ui| (ui.button("Save").clicked(), ui.button("Cancel").clicked()))
                        .inner
                });
                let (save, cancel) = resp.inner;
                if save {
                    let (n, s) = (name.clone(), shortcut.clone());
                    match self.save_recorded(&n, &s) {
                        Ok(()) => close = true,
                        Err(e) => {
                            if let Some(Dialog::Save { error, .. }) = &mut self.dialog {
                                *error = Some(e);
                            }
                        }
                    }
                }
                close |= cancel || resp.should_close();
            }
            Some(Dialog::Run {
                source,
                times,
                until_end,
            }) => {
                let names: Vec<String> = std::iter::once("Recorded macro".to_owned())
                    .chain(self.saved.iter().map(|m| m.name.clone()))
                    .collect();
                let label = |s: MacroSource| match s {
                    MacroSource::Recorded => names[0].clone(),
                    MacroSource::Saved(i) => names.get(i + 1).cloned().unwrap_or_default(),
                };
                let saved_count = self.saved.len();
                let resp = Modal::new(Id::new("ochrepad-run-macro")).show(ui.ctx(), |ui| {
                    ui.heading("Run a macro multiple times");
                    egui::ComboBox::from_label("Macro")
                        .selected_text(label(*source))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                source,
                                MacroSource::Recorded,
                                label(MacroSource::Recorded),
                            );
                            for i in 0..saved_count {
                                let s = MacroSource::Saved(i);
                                ui.selectable_value(source, s, label(s));
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.radio_value(until_end, false, "Repeat");
                        ui.add_enabled(
                            !*until_end,
                            egui::DragValue::new(times).range(1..=1_000_000),
                        );
                        ui.label("times");
                    });
                    ui.radio_value(until_end, true, "Run until the end of the file");
                    ui.horizontal(|ui| (ui.button("Run").clicked(), ui.button("Cancel").clicked()))
                        .inner
                });
                let (go, cancel) = resp.inner;
                if go {
                    let repeat = if *until_end {
                        Repeat::UntilEnd
                    } else {
                        Repeat::Times(*times)
                    };
                    run = Some((*source, repeat));
                    close = true;
                }
                close |= cancel || resp.should_close();
            }
        }
        if close {
            self.dialog = None;
        }
        run
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse() {
        assert_eq!(
            parse_shortcut("Ctrl+Shift+1"),
            Some(KeyboardShortcut::new(
                Modifiers::COMMAND.plus(Modifiers::SHIFT),
                Key::Num1
            ))
        );
        assert_eq!(
            parse_shortcut("alt + f5"),
            Some(KeyboardShortcut::new(Modifiers::ALT, Key::F5))
        );
        assert_eq!(
            parse_shortcut("Ctrl+K").map(|s| s.logical_key),
            Some(Key::K)
        );
        assert_eq!(parse_shortcut("Hyper+K"), None);
        assert_eq!(parse_shortcut("Ctrl+"), None);
    }
}
