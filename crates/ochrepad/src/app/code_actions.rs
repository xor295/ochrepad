//! Code actions (Ctrl+.; PLAN_completion L3): the language server's fixes
//! and refactorings for the selection or the caret, in a menu at the caret.
//! An action's edit is applied like Rename's (checked first, one undo step
//! per document); a command runs on the server only, and only a command the
//! server says it has. While it runs, the server may ask ochrepad to apply
//! an edit (`workspace/applyEdit`); at any other time that is refused.

use std::time::Duration;

use egui::{Align2, Id, Key, Modifiers, Pos2, Sense, Ui, Vec2};
use ochrepad_core::BufferId;
use ochrepad_core::position::ColumnUnits;
use ochrepad_lsp::Tag;
use serde_json::{Value, json};

use super::OchrepadApp;

/// Where Code Actions is.
pub(super) enum CodeActions {
    /// Asking the server for the actions.
    Asking {
        buffer: BufferId,
        tag: Tag,
        units: ColumnUnits,
    },
    /// The menu.
    Showing {
        buffer: BufferId,
        units: ColumnUnits,
        items: Vec<Item>,
        selected: usize,
        at: Pos2,
    },
    /// Asking the server for an action's edit (`codeAction/resolve`).
    Resolving {
        buffer: BufferId,
        tag: Tag,
        units: ColumnUnits,
    },
    /// The server runs an action's command; it may ask for edits.
    Running {
        buffer: BufferId,
        tag: Tag,
        title: String,
    },
}

pub(super) struct Item {
    title: String,
    /// Why it cannot be chosen now.
    disabled: Option<String>,
    preferred: bool,
    /// The `CodeAction` or `Command` as sent.
    raw: Value,
}

impl OchrepadApp {
    /// The titles in the code action menu, while it shows (tests).
    pub fn code_action_titles(&self) -> Vec<String> {
        match &self.code_actions {
            Some(CodeActions::Showing { items, .. }) => {
                items.iter().map(|i| i.title.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Ctrl+.: asks for the actions at the selection.
    pub(super) fn code_actions(&mut self) {
        let buffer = self.buffer_id();
        let (Some(doc), Some(sel)) = (
            self.editor.buffer(buffer).map(|b| b.doc().clone()),
            self.editor
                .view(self.active)
                .map(|v| v.selections.primary()),
        ) else {
            return;
        };
        let Some(units) = self
            .language_servers
            .session_of(buffer)
            .map(|s| s.client.units)
        else {
            self.note_text("No language server works on this document (View > Language Servers)");
            return;
        };
        let range = sel.range();
        // The problems there, as the server sent them (its fixes may need
        // what they carry).
        let diagnostics: Vec<Value> = self
            .language_servers
            .diagnostics_of(buffer)
            .iter()
            .filter(|d| {
                d.range
                    .as_ref()
                    .is_some_and(|r| r.start <= range.end && range.start <= r.end)
            })
            .map(|d| d.raw.clone())
            .collect();
        let sent = self.language_servers.request(
            buffer,
            &doc,
            "codeActionProvider",
            "textDocument/codeAction",
            |c, uri| {
                json!({
                    "textDocument": { "uri": uri },
                    "range": { "start": c.position(&doc, range.start), "end": c.position(&doc, range.end) },
                    "context": { "diagnostics": diagnostics, "triggerKind": 1 },
                })
            },
            Duration::from_secs(10),
        );
        match sent {
            Some((tag, _)) => self.code_actions = Some(CodeActions::Asking { buffer, tag, units }),
            None => self.note_text("The language server has no code actions"),
        }
    }

    /// Once a frame: the server's answers.
    pub(super) fn code_actions_tick(&mut self) {
        let tag = match &self.code_actions {
            Some(
                CodeActions::Asking { tag, .. }
                | CodeActions::Resolving { tag, .. }
                | CodeActions::Running { tag, .. },
            ) => *tag,
            _ => return,
        };
        let Some(answer) = self.language_servers.take_answer(tag) else {
            return;
        };
        match self.code_actions.take() {
            Some(CodeActions::Asking { buffer, units, .. }) => {
                let items = match answer {
                    Ok(Value::Array(a)) => a.iter().filter_map(item).collect::<Vec<_>>(),
                    Ok(_) => Vec::new(),
                    Err(e) => {
                        self.note_text(format!("Code actions: {}", e.message));
                        return;
                    }
                };
                if items.is_empty() {
                    self.note_text("No code actions here");
                    return;
                }
                let at = self
                    .view_state()
                    .and_then(|s| s.caret_rect)
                    .map_or(Pos2::new(200.0, 200.0), |r| r.left_bottom());
                let selected = items
                    .iter()
                    .position(|i| i.preferred && i.disabled.is_none())
                    .or_else(|| items.iter().position(|i| i.disabled.is_none()))
                    .unwrap_or(0);
                self.code_actions = Some(CodeActions::Showing {
                    buffer,
                    units,
                    items,
                    selected,
                    at,
                });
            }
            Some(CodeActions::Resolving { buffer, units, .. }) => match answer {
                Ok(action) => self.run_action(buffer, units, &action, false),
                Err(e) => self.note_text(format!("Code action: {}", e.message)),
            },
            Some(CodeActions::Running { buffer, title, .. }) => {
                self.language_servers.allow_apply_edit(buffer, false);
                if let Err(e) = answer {
                    self.note_text(format!("{title}: {}", e.message));
                }
            }
            other => self.code_actions = other,
        }
    }

    /// Runs a chosen action: its edit, then its command (resolving it
    /// first when it came with neither and `may_resolve`).
    fn run_action(
        &mut self,
        buffer: BufferId,
        units: ColumnUnits,
        action: &Value,
        may_resolve: bool,
    ) {
        let is_command = action.get("command").is_some_and(Value::is_string);
        if !is_command
            && action.get("edit").is_none()
            && action.get("command").is_none()
            && may_resolve
            && self.language_servers.session_of(buffer).is_some_and(|s| {
                s.client
                    .capabilities
                    .pointer("/codeActionProvider/resolveProvider")
                    .and_then(Value::as_bool)
                    == Some(true)
            })
        {
            let raw = action.clone();
            let doc = self.editor.buffer(buffer).map(|b| b.doc().clone());
            if let Some(doc) = doc
                && let Some((tag, _)) = self.language_servers.request(
                    buffer,
                    &doc,
                    "codeActionProvider",
                    "codeAction/resolve",
                    |_, _| raw,
                    Duration::from_secs(10),
                )
            {
                self.code_actions = Some(CodeActions::Resolving { buffer, tag, units });
            }
            return;
        }
        if let Some(edit) = action.get("edit")
            && let Err(e) = self.apply_workspace_edit(edit, units)
        {
            self.message = Some(format!("The code action was not applied:\n{e}"));
            return;
        }
        // A bare `Command`, or an action's.
        let command = if is_command {
            Some(action)
        } else {
            action.get("command")
        };
        if let Some(c) = command {
            self.run_command(buffer, c);
        }
    }

    /// Asks the server to run `command`, if it says it has it.
    fn run_command(&mut self, buffer: BufferId, command: &Value) {
        let Some(name) = command.get("command").and_then(Value::as_str) else {
            return;
        };
        let title = command
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_owned();
        let known = self.language_servers.session_of(buffer).is_some_and(|s| {
            s.client
                .capabilities
                .pointer("/executeCommandProvider/commands")
                .and_then(Value::as_array)
                .is_some_and(|c| c.iter().any(|c| c.as_str() == Some(name)))
        });
        if !known {
            // Commands an editor extension would run (a test, a debugger).
            self.note_text(format!("{title}: ochrepad cannot run {name}"));
            return;
        }
        let Some(doc) = self.editor.buffer(buffer).map(|b| b.doc().clone()) else {
            return;
        };
        let params = json!({
            "command": name,
            "arguments": command.get("arguments").cloned().unwrap_or(json!([])),
        });
        let sent = self.language_servers.request(
            buffer,
            &doc,
            "executeCommandProvider",
            "workspace/executeCommand",
            |_, _| params,
            Duration::from_secs(30),
        );
        if let Some((tag, _)) = sent {
            self.language_servers.allow_apply_edit(buffer, true);
            self.code_actions = Some(CodeActions::Running { buffer, tag, title });
        }
    }

    /// The menu's keys, before the editor sees them: whether it took them.
    pub(super) fn code_actions_keys(&mut self, ui: &mut Ui) {
        let Some(CodeActions::Showing {
            items, selected, ..
        }) = &mut self.code_actions
        else {
            return;
        };
        let n = items.len();
        let (down, up, enter, escape) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down {
            *selected = (*selected + 1) % n;
        }
        if up {
            *selected = (*selected + n - 1) % n;
        }
        if escape {
            self.code_actions = None;
        } else if enter {
            self.choose_action(None);
        }
    }

    /// Chooses the selected action (or `index`).
    fn choose_action(&mut self, index: Option<usize>) {
        let Some(CodeActions::Showing {
            buffer,
            units,
            items,
            selected,
            ..
        }) = self.code_actions.take()
        else {
            return;
        };
        let Some(item) = items.get(index.unwrap_or(selected)) else {
            return;
        };
        if let Some(reason) = &item.disabled {
            self.note_text(format!("{}: {reason}", item.title));
            return;
        }
        let raw = item.raw.clone();
        self.run_action(buffer, units, &raw, true);
    }

    /// The menu at the caret.
    pub(super) fn code_actions_menu(&mut self, ui: &mut Ui) {
        let Some(CodeActions::Showing {
            items,
            selected,
            at,
            ..
        }) = &self.code_actions
        else {
            return;
        };
        let mut clicked = None;
        let mut outside = false;
        let shown = egui::Area::new(Id::new("ochrepad-code-actions"))
            .order(egui::Order::Foreground)
            .fixed_pos(*at)
            .pivot(Align2::LEFT_TOP)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(240.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, item) in items.iter().enumerate() {
                        let text = egui::RichText::new(&item.title);
                        let text = if item.disabled.is_some() {
                            text.weak()
                        } else {
                            text
                        };
                        let r = ui.add(
                            egui::Button::selectable(i == *selected, text)
                                .min_size(Vec2::new(ui.available_width(), 0.0))
                                .sense(Sense::click()),
                        );
                        let r = match &item.disabled {
                            Some(reason) => r.on_hover_text(reason),
                            None => r,
                        };
                        if r.clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        if ui.input(|i| i.pointer.any_pressed())
            && !shown.response.contains_pointer()
            && clicked.is_none()
        {
            outside = true;
        }
        if let Some(i) = clicked {
            self.choose_action(Some(i));
        } else if outside {
            self.code_actions = None;
        }
    }
}

/// A `CodeAction` or `Command` from the server's list.
fn item(v: &Value) -> Option<Item> {
    let title = v.get("title")?.as_str()?;
    if title.is_empty() {
        return None;
    }
    Some(Item {
        title: title.lines().next().unwrap_or(title).to_owned(),
        disabled: v
            .get("disabled")
            .and_then(|d| d.get("reason"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        preferred: v.get("isPreferred").and_then(Value::as_bool) == Some(true),
        raw: v.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_and_commands_from_json() {
        let a = item(
            &json!({"title": "Add `use`", "kind": "quickfix", "isPreferred": true, "edit": {}}),
        )
        .unwrap();
        assert!(a.preferred);
        let c = item(&json!({"title": "Run", "command": "x.run"})).unwrap();
        assert_eq!(c.title, "Run");
        let d = item(&json!({"title": "Extract", "disabled": {"reason": "Select an expression"}}))
            .unwrap();
        assert_eq!(d.disabled.as_deref(), Some("Select an expression"));
        assert!(item(&json!({"command": "x"})).is_none());
    }
}
