//! The command palette (Ctrl+Shift+A): every command by name, found by
//! typing a few of its letters. It is an overlay near the top of the
//! window, not part of a bar, so it is the same whatever has the focus.
//!
//! The commands are listed when the palette opens (`commands`), named by
//! their menu path ("Edit: Line Operations: Join Lines"), so the list can
//! leave out what does not apply (editing commands while a viewer or a
//! terminal has the focus) and take in what changes (saved macros,
//! languages). The menus are drawn by their own code: a command added to a
//! menu is added here too.

use egui::text::LayoutJob;
use egui::{
    Align2, Area, FontSelection, Frame, Id, Key, KeyboardShortcut, Modifiers, Order, Rect,
    RichText, ScrollArea, Sense, TextEdit, TextFormat, TextStyle, Ui, ViewportCommand, WidgetInfo,
    WidgetType, pos2, vec2,
};
use ochrepad_core::complete::Pattern;
use ochrepad_core::{Action, CaseMode, Codec, Repeat, SortOptions};

use super::edit_tools::TabFileCommand;
use super::{MacroSource, OchrepadApp, ThemeChoice};
use crate::editor_view::copy_selection;
use crate::keymap::EditorCommand;

/// Ctrl+Shift+A, "Find Action" in JetBrains' editors. (Ctrl+Shift+P, other
/// editors' key for this, plays the recorded macro, as in Notepad++.)
pub(super) const OPEN_PALETTE: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::A);
pub(super) const OPEN_PALETTE_TEXT: &str = "Ctrl+Shift+A";

/// Rows shown before the list scrolls.
const ROWS: usize = 12;
const WIDTH: f32 = 560.0;
/// Commands run lately that an empty palette lists first.
const RECENT: usize = 8;

type Run = Box<dyn Fn(&mut OchrepadApp, &Ui)>;

struct Command {
    name: String,
    shortcut: String,
    /// For a setting that is on or off: which it is now.
    on: Option<bool>,
    run: Run,
}

/// A command the list shows, and the chars of its name the typed text
/// matched.
struct Shown {
    command: usize,
    matched: Vec<usize>,
}

struct Open {
    query: String,
    commands: Vec<Command>,
    shown: Vec<Shown>,
    selected: usize,
    /// The first frame: the field takes the keyboard, and the click that
    /// opened the palette is not a click outside it.
    first: bool,
    /// Scroll the selected row into view.
    reveal: bool,
    scroll: f32,
    /// Run the selected command (Enter).
    run: bool,
}

#[derive(Default)]
pub(super) struct Palette {
    open: Option<Open>,
    /// Names of the commands run lately, the latest first.
    recent: Vec<String>,
}

impl Open {
    /// Lists the commands the typed text matches, best first; all of them
    /// when nothing is typed, the ones run lately first.
    fn filter(&mut self, recent: &[String]) {
        // Spaces separate what is typed; they are not looked for.
        let typed: String = self.query.chars().filter(|c| !c.is_whitespace()).collect();
        self.selected = 0;
        self.reveal = true;
        if typed.is_empty() {
            let place = |c: &Command| {
                recent
                    .iter()
                    .position(|r| *r == c.name)
                    .unwrap_or(usize::MAX)
            };
            let mut order: Vec<usize> = (0..self.commands.len()).collect();
            order.sort_by_key(|&i| place(&self.commands[i])); // stable
            self.shown = order
                .into_iter()
                .map(|command| Shown {
                    command,
                    matched: Vec::new(),
                })
                .collect();
            return;
        }
        let pattern = Pattern::new(&typed);
        let mut found: Vec<(i64, Shown)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(command, c)| {
                let (score, matched) = pattern.positions(&c.name)?;
                Some((score, Shown { command, matched }))
            })
            .collect();
        found.sort_by_key(|(score, _)| std::cmp::Reverse(*score)); // stable
        self.shown = found.into_iter().map(|(_, s)| s).collect();
    }
}

/// Collects the commands for [`OchrepadApp::palette_commands`].
struct Commands<'a> {
    app: &'a OchrepadApp,
    out: Vec<Command>,
}

impl Commands<'_> {
    fn add(
        &mut self,
        name: impl Into<String>,
        shortcut: &str,
        run: impl Fn(&mut OchrepadApp, &Ui) + 'static,
    ) {
        self.out.push(Command {
            name: name.into(),
            shortcut: shortcut.to_owned(),
            on: None,
            run: Box::new(run),
        });
    }

    /// An editing action, with the key the keymap gives it now.
    fn act(&mut self, name: impl Into<String>, action: Action) {
        let shortcut = self
            .app
            .keymap
            .chord_for(&EditorCommand::Action(action.clone()))
            .map(|c| c.to_string())
            .unwrap_or_default();
        self.out.push(Command {
            name: name.into(),
            shortcut,
            on: None,
            run: Box::new(move |app, _| app.apply(action.clone())),
        });
    }

    /// A setting that is on or off.
    fn toggle(
        &mut self,
        name: impl Into<String>,
        shortcut: &str,
        on: bool,
        run: impl Fn(&mut OchrepadApp, &Ui) + 'static,
    ) {
        self.add(name, shortcut, run);
        if let Some(c) = self.out.last_mut() {
            c.on = Some(on);
        }
    }
}

/// Any seed for the random commands: the time will do.
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

impl OchrepadApp {
    /// Opens the command palette (or closes it, if it is open).
    pub fn toggle_palette(&mut self) {
        if self.palette.open.take().is_some() {
            return;
        }
        // What has the focus decides which commands apply.
        let viewer = self.focused_viewer();
        let terminal = self.focused_terminal().is_some();
        let mut open = Open {
            query: String::new(),
            commands: self.palette_commands(viewer, terminal),
            shown: Vec::new(),
            selected: 0,
            first: true,
            reveal: true,
            scroll: 0.0,
            run: false,
        };
        open.filter(&self.palette.recent);
        self.palette.open = Some(open);
    }

    pub fn palette_is_open(&self) -> bool {
        self.palette.open.is_some()
    }

    /// The names the palette lists now, best first (tests).
    pub fn palette_names(&self) -> Vec<String> {
        self.palette.open.as_ref().map_or_else(Vec::new, |o| {
            o.shown
                .iter()
                .map(|s| o.commands[s.command].name.clone())
                .collect()
        })
    }

    /// Early in the frame, before anything else reads the keyboard: the
    /// palette's own key, and while it is open the keys that move in its
    /// list, run a command or close it.
    pub(super) fn palette_keys(&mut self, ui: &mut Ui) {
        let toggled = ui.input_mut(|i| !i.modifiers.alt && i.consume_shortcut(&OPEN_PALETTE));
        if toggled {
            self.toggle_palette();
        }
        let Some(o) = &mut self.palette.open else {
            return;
        };
        let last = o.shown.len().saturating_sub(1);
        let page = ROWS - 1;
        let mut close = false;
        let before = o.selected;
        ui.input_mut(|i| {
            let mut key = |key| i.consume_key(Modifiers::NONE, key);
            if key(Key::Escape) {
                close = true;
            }
            if key(Key::ArrowDown) {
                o.selected = if o.selected >= last {
                    0
                } else {
                    o.selected + 1
                };
            }
            if key(Key::ArrowUp) {
                o.selected = if o.selected == 0 {
                    last
                } else {
                    o.selected - 1
                };
            }
            if key(Key::PageDown) {
                o.selected = (o.selected + page).min(last);
            }
            if key(Key::PageUp) {
                o.selected = o.selected.saturating_sub(page);
            }
            if key(Key::Enter) {
                o.run = true;
            }
        });
        o.reveal |= o.selected != before;
        if close {
            self.palette.open = None;
        }
    }

    /// Draws the palette over the window, and runs the command chosen.
    pub(super) fn palette_ui(&mut self, ui: &mut Ui) {
        let recent = &self.palette.recent;
        let Some(o) = &mut self.palette.open else {
            return;
        };
        let screen = ui.ctx().content_rect();
        let width = WIDTH.min(screen.width() - 32.0).max(120.0);
        // Below the menu bar and the tabs, over the text.
        let top = pos2(screen.center().x, screen.top() + 2.0 * crate::theme::BAR_H);
        let mut clicked = None;
        let area = Area::new(Id::new("ochrepad-palette"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(top)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    let field = ui.add(
                        TextEdit::singleline(&mut o.query)
                            .id(Id::new("ochrepad-palette-field"))
                            .hint_text("Type a command's name")
                            .desired_width(f32::INFINITY),
                    );
                    // The keyboard stays here while the palette is open.
                    if !field.has_focus() {
                        field.request_focus();
                    }
                    if field.changed() {
                        o.filter(recent);
                    }
                    ui.add_space(4.0);
                    if o.shown.is_empty() {
                        ui.label(RichText::new("No command matches").weak());
                    } else {
                        clicked = rows(ui, o);
                    }
                });
            });
        let outside = !o.first && area.response.clicked_elsewhere();
        o.first = false;
        let run = clicked.or_else(|| {
            std::mem::take(&mut o.run)
                .then(|| o.shown.get(o.selected).map(|s| s.command))
                .flatten()
        });
        if let Some(command) = run {
            let Some(mut o) = self.palette.open.take() else {
                return;
            };
            let command = o.commands.swap_remove(command);
            self.palette.recent.retain(|r| *r != command.name);
            self.palette.recent.insert(0, command.name.clone());
            self.palette.recent.truncate(RECENT);
            (command.run)(self, ui);
        } else if outside {
            self.palette.open = None;
        }
    }

    /// The button at the right end of the menu bar.
    pub(super) fn palette_button(&mut self, ui: &mut Ui) {
        // Not in a window too narrow for it beside the menus.
        if ui.available_width() < 120.0 {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new("Commands").weak())
                .on_hover_text(format!(
                    "Find any command by typing part of its name ({OPEN_PALETTE_TEXT})"
                ))
                .clicked()
            {
                self.toggle_palette();
            }
        });
    }

    /// Every command that applies now, in the menus' order.
    fn palette_commands(
        &self,
        viewer: Option<crate::viewer::ViewerId>,
        terminal: bool,
    ) -> Vec<Command> {
        // Editing commands need an editor with the focus.
        let editing = viewer.is_none() && !terminal;
        let named = editing && self.buffer().meta.path.is_some();
        let buffer = self.buffer_id();
        let mut c = Commands {
            app: self,
            out: Vec::new(),
        };

        // --- File ----------------------------------------------------------
        c.add("File: New", "Ctrl+N", |a, _| {
            a.new_file();
        });
        c.add("File: Open…", "Ctrl+O", |a, _| a.open_dialog());
        if !self.closed_files.is_empty() {
            c.add(
                "File: Restore Recent Closed File",
                "Ctrl+Shift+T",
                |a, _| a.restore_closed_file(),
            );
        }
        c.add("File: Open Folder…", "", |a, _| a.open_folder_dialog());
        if self.tree.is_some() {
            c.add("File: Close Folder", "", |a, _| a.close_folder());
        }
        c.add("File: Load Session…", "", |a, _| a.load_session());
        c.add("File: Save Session…", "", |a, _| a.save_session());
        if editing {
            c.add("File: Save", "Ctrl+S", |a, _| {
                a.save();
            });
            c.add("File: Save As…", "Ctrl+Shift+S", |a, _| {
                a.save_as();
            });
            c.add("File: Save a Copy As…", "", |a, _| a.save_copy_as());
            c.add("File: Export: HTML…", "", |a, _| a.export_html());
            c.add("File: Save All", "", |a, _| a.save_all());
        }
        if named {
            c.add("File: Reload from Disk", "", |a, _| {
                a.tab_file_command(TabFileCommand::Reload, a.active)
            });
            c.add("File: Rename…", "", |a, _| a.open_rename());
        }
        c.add("File: Close", "Ctrl+W", |a, _| {
            match (a.focused_terminal(), a.focused_viewer()) {
                (Some(id), _) => a.close_terminal(id),
                (None, Some(id)) => a.close_viewer(id),
                (None, None) => a.close_view(a.active),
            }
        });
        if editing {
            c.add("File: Close All but Active", "", |a, _| {
                a.close_others(a.active)
            });
        }
        c.add("File: Close All", "", |a, _| a.close_all());
        c.add("File: Exit", "", |a, _| a.request_exit());

        // --- Edit ----------------------------------------------------------
        if viewer.is_some() {
            c.add("Edit: Copy", "Ctrl+C", |a, ui| {
                if let Some(v) = a.focused_viewer().and_then(|id| a.viewers.get_mut(id)) {
                    v.state.copy(ui.ctx(), &v.file);
                }
            });
        }
        if editing {
            c.act("Edit: Undo", Action::Undo);
            c.act("Edit: Redo", Action::Redo);
            c.add("Edit: Clear Undo History", "", |a, _| {
                a.clear_undo_history()
            });
            c.add("Edit: Cut", "Ctrl+X", |a, ui| {
                if copy_selection(ui.ctx(), &a.editor, a.active) {
                    a.apply(Action::InsertText(String::new()));
                }
            });
            c.add("Edit: Copy", "Ctrl+C", |a, ui| {
                copy_selection(ui.ctx(), &a.editor, a.active);
            });
            c.add("Edit: Paste", "Ctrl+V", |a, ui| {
                // Answered with a paste event, which the editor handles
                // once it has the keyboard again.
                ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
                a.view_state_mut().request_focus();
            });
            c.act("Edit: Select All", Action::SelectAll);
            for (name, column) in [
                ("Edit: Begin/End Select", false),
                ("Edit: Begin/End Select in Column Mode", true),
            ] {
                c.add(name, "", move |a, _| {
                    let view = a.active;
                    let state = a.view_states.entry(view).or_default();
                    crate::editor_view::begin_end_select(&mut a.editor, view, state, column);
                });
            }
            for (title, all) in [("Multi-select All", true), ("Multi-select Next", false)] {
                for (label, match_case, whole_word) in [
                    ("Ignore Case & Whole Word", false, false),
                    ("Match Case Only", true, false),
                    ("Match Whole Word Only", false, true),
                    ("Match Case & Whole Word", true, true),
                ] {
                    c.act(
                        format!("Edit: {title}: {label}"),
                        if all {
                            Action::MultiSelectAll {
                                match_case,
                                whole_word,
                            }
                        } else {
                            Action::MultiSelectNext {
                                match_case,
                                whole_word,
                            }
                        },
                    );
                }
            }
            for (label, action) in [
                ("Toggle Comment", Action::ToggleLineComment),
                ("Comment Lines", Action::CommentLines),
                ("Uncomment Lines", Action::UncommentLines),
                ("Toggle Block Comment", Action::ToggleBlockComment),
            ] {
                c.act(format!("Edit: Comment/Uncomment: {label}"), action);
            }
            c.act("Edit: Indent", Action::Indent);
            c.act("Edit: Unindent", Action::Unindent);
            for (label, mode) in [
                ("UPPERCASE", CaseMode::Upper),
                ("lowercase", CaseMode::Lower),
                ("Title Case", CaseMode::Title),
                ("Title Case (blend)", CaseMode::TitleBlend),
                ("Sentence case", CaseMode::Sentence),
                ("Sentence case (blend)", CaseMode::SentenceBlend),
                ("iNVERT cASE", CaseMode::Invert),
            ] {
                c.act(
                    format!("Edit: Convert Case: {label}"),
                    Action::ConvertCase(mode),
                );
            }
            c.add("Edit: Convert Case: ranDOm cASE", "", |a, _| {
                a.apply(Action::ConvertCase(CaseMode::Random { seed: seed() }))
            });
            c.add(
                "Edit: Rename… (a name, everywhere)",
                "Shift+F6",
                |a, _| a.rename_symbol(),
            );
            c.add("Edit: Format Document", "Alt+Shift+F", |a, _| {
                a.format_document()
            });
            c.add("Edit: Code Actions…", "Ctrl+.", |a, _| a.code_actions());

            for (label, action) in [
                ("Duplicate Current Line", Action::DuplicateLine),
                ("Move Up Current Line", Action::MoveLinesUp),
                ("Move Down Current Line", Action::MoveLinesDown),
                ("Join Lines", Action::JoinLines),
                ("Delete Line", Action::DeleteLine),
                ("Transpose Line", Action::TransposeLines),
                ("Insert Blank Line Above Current", Action::InsertLineAbove),
                ("Insert Blank Line Below Current", Action::InsertLineBelow),
                ("Reverse Line Order", Action::ReverseLines),
            ] {
                c.act(format!("Edit: Line Operations: {label}"), action);
            }
            c.add("Edit: Line Operations: Split Lines", "Ctrl+I", |a, _| {
                let width = a.view_state().map_or(80, |s| s.split_width());
                a.apply(Action::SplitLines { width });
            });
            for (label, action) in [
                (
                    "Remove Empty Lines",
                    Action::RemoveEmptyLines {
                        include_whitespace: false,
                    },
                ),
                (
                    "Remove Empty Lines (Containing Blank characters)",
                    Action::RemoveEmptyLines {
                        include_whitespace: true,
                    },
                ),
                (
                    "Remove Duplicate Lines",
                    Action::RemoveDuplicateLines {
                        consecutive_only: false,
                    },
                ),
                (
                    "Remove Consecutive Duplicate Lines",
                    Action::RemoveDuplicateLines {
                        consecutive_only: true,
                    },
                ),
            ] {
                c.act(format!("Edit: Line Operations: {label}"), action);
            }
            let sort = |descending, case_insensitive, numeric, remove_duplicates, by_length| {
                Action::SortLines(SortOptions {
                    descending,
                    case_insensitive,
                    numeric,
                    remove_duplicates,
                    by_length,
                })
            };
            for (label, action) in [
                (
                    "Sort Lines Ascending",
                    sort(false, false, false, false, false),
                ),
                (
                    "Sort Lines Descending",
                    sort(true, false, false, false, false),
                ),
                (
                    "Sort Lines Ascending (Ignore Case)",
                    sort(false, true, false, false, false),
                ),
                (
                    "Sort Lines Descending (Ignore Case)",
                    sort(true, true, false, false, false),
                ),
                (
                    "Sort Lines as Numbers Ascending",
                    sort(false, false, true, false, false),
                ),
                (
                    "Sort Lines as Numbers Descending",
                    sort(true, false, true, false, false),
                ),
                (
                    "Sort Lines and Remove Duplicates",
                    sort(false, false, false, true, false),
                ),
                (
                    "Sort Lines by Length Ascending",
                    sort(false, false, false, false, true),
                ),
                (
                    "Sort Lines by Length Descending",
                    sort(true, false, false, false, true),
                ),
            ] {
                c.act(format!("Edit: Line Operations: {label}"), action);
            }
            c.add("Edit: Line Operations: Randomize Line Order", "", |a, _| {
                a.apply(Action::ShuffleLines { seed: seed() })
            });

            for (label, action) in [
                ("Trim Trailing Space", Action::TrimTrailingWhitespace),
                ("Trim Leading Space", Action::TrimLeadingWhitespace),
                (
                    "Trim Leading and Trailing Space",
                    Action::TrimLeadingAndTrailingWhitespace,
                ),
                ("EOL to Space", Action::EolToSpace),
                ("TAB to Space", Action::TabsToSpaces),
                ("Space to TAB (All)", Action::SpacesToTabs),
                (
                    "Space to TAB (Leading)",
                    Action::ConvertIndentation { to_spaces: false },
                ),
            ] {
                c.act(format!("Edit: Blank Operations: {label}"), action);
            }

            c.act(
                "Edit: Text Tools: Format JSON",
                Action::FormatJson { indent: None },
            );
            c.act("Edit: Text Tools: Minify JSON", Action::MinifyJson);
            c.add("Edit: Text Tools: Validate JSON", "", |a, _| {
                a.validate_json()
            });
            for (label, action) in [
                ("Base64 Encode", Action::Encode(Codec::Base64)),
                ("Base64 Decode", Action::Decode(Codec::Base64)),
                ("URL Encode", Action::Encode(Codec::Url)),
                ("URL Decode", Action::Decode(Codec::Url)),
                ("Hex Encode", Action::Encode(Codec::Hex)),
                ("Hex Decode", Action::Decode(Codec::Hex)),
            ] {
                c.act(format!("Edit: Text Tools: {label}"), action);
            }
            c.add("Edit: Text Tools: Hashes…", "", |a, _| a.show_hashes());
            c.add("Edit: Column Editor…", "Alt+C", |a, _| {
                a.open_column_editor()
            });
            c.toggle("Edit: Read-Only", "", self.buffer().read_only, |a, _| {
                let buffer = a.buffer_id();
                if let Some(b) = a.editor.buffer_mut(buffer) {
                    b.read_only = !b.read_only;
                }
            });
        }

        // --- Search --------------------------------------------------------
        c.add("Search: Find…", "Ctrl+F", |a, _| {
            match a.focused_viewer() {
                Some(id) => a.open_find_on_viewer(id),
                None => a.find.open(&a.editor, a.active, false),
            }
        });
        if editing {
            c.add("Search: Replace…", "Ctrl+H", |a, _| {
                a.find.open(&a.editor, a.active, true)
            });
        }
        c.add("Search: Go to Line…", "Ctrl+G", |a, _| {
            a.go_to_line = Some(String::new())
        });
        if editing {
            c.add("Search: Go to Definition", "F12", |a, _| {
                a.go_to_definition()
            });
            c.add("Search: Find References", "Shift+F12", |a, _| {
                a.find_references()
            });
        }
        c.add("Search: Go to Symbol in Folder…", "Ctrl+Alt+T", |a, _| {
            a.go_to_symbol()
        });
        c.add("Search: Filter Lines…", "Alt+Shift+L", |a, _| {
            a.open_filter()
        });
        if editing {
            c.act(
                "Search: Go to Matching Bracket",
                Action::GoToMatchingBracket,
            );
            c.act(
                "Search: Select to Matching Bracket",
                Action::SelectToMatchingBracket,
            );
        }
        c.add("Search: Find in Files…", "Ctrl+Shift+F", |a, _| {
            a.open_find_in_files(None)
        });
        if editing {
            c.add("Search: Select and Find Next", "Ctrl+F3", |a, _| {
                a.select_and_find(false)
            });
            c.add(
                "Search: Select and Find Previous",
                "Ctrl+Shift+F3",
                |a, _| a.select_and_find(true),
            );
        }
        for (name, shortcut, backward) in [
            ("Search: Find Next", "F3", false),
            ("Search: Find Previous", "Shift+F3", true),
        ] {
            c.add(name, shortcut, move |a, _| a.palette_find(backward));
        }
        if editing {
            for (i, style) in crate::token_styles::NAMES.iter().enumerate() {
                c.add(
                    format!("Search: Style All Occurrences of Token: Using {style} Style"),
                    "",
                    move |a, _| a.style_token(i),
                );
            }
            for (i, style) in crate::token_styles::NAMES.iter().enumerate() {
                if !self.token_styles.has(buffer, i) {
                    continue;
                }
                c.add(
                    format!("Search: Clear Style: Clear {style} Style"),
                    "",
                    move |a, _| {
                        let buffer = a.buffer_id();
                        a.token_styles.clear(buffer, Some(i));
                    },
                );
                for (title, backward) in [("Jump Up", true), ("Jump Down", false)] {
                    c.add(
                        format!("Search: {title}: {style} Style"),
                        "",
                        move |a, _| a.jump_to_style(i, backward),
                    );
                }
                c.add(
                    format!("Search: Copy Styled Text: Using {style} Style"),
                    "",
                    move |a, ui| a.copy_styled_text(ui, Some(i)),
                );
            }
            c.add("Search: Clear Style: Clear All Styles", "", |a, _| {
                let buffer = a.buffer_id();
                a.token_styles.clear(buffer, None);
            });
            c.add("Search: Copy Styled Text: Using All Styles", "", |a, ui| {
                a.copy_styled_text(ui, None)
            });

            c.act("Search: Bookmark: Toggle Bookmark", Action::ToggleBookmark);
            c.act(
                "Search: Bookmark: Inverse Bookmark",
                Action::InverseBookmarks,
            );
            if !self.buffer().bookmarks().is_empty() {
                c.act("Search: Bookmark: Next Bookmark", Action::NextBookmark);
                c.act("Search: Bookmark: Previous Bookmark", Action::PrevBookmark);
                c.act(
                    "Search: Bookmark: Clear All Bookmarks",
                    Action::ClearBookmarks,
                );
                c.add("Search: Bookmark: Cut Bookmarked Lines", "", |a, ui| {
                    a.copy_bookmarked_lines(ui.ctx());
                    a.apply(Action::RemoveBookmarkedLines);
                });
                c.add("Search: Bookmark: Copy Bookmarked Lines", "", |a, ui| {
                    a.copy_bookmarked_lines(ui.ctx())
                });
                c.add(
                    "Search: Bookmark: Paste to (Replace) Bookmarked Lines",
                    "",
                    |a, ui| {
                        let now = a.now;
                        a.view_state_mut().paste_to_bookmarks(now);
                        ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
                        a.view_state_mut().request_focus();
                    },
                );
                c.act(
                    "Search: Bookmark: Remove Bookmarked Lines",
                    Action::RemoveBookmarkedLines,
                );
                c.act(
                    "Search: Bookmark: Remove Unmarked Lines",
                    Action::RemoveUnbookmarkedLines,
                );
            }
        }

        // --- Macro ---------------------------------------------------------
        let recording = self.editor.is_recording();
        let has_recording = !self.macros.recorded.is_empty();
        c.add(
            if recording {
                "Macro: Stop Recording"
            } else {
                "Macro: Start Recording"
            },
            "Ctrl+Shift+R",
            |a, _| a.macros.toggle_recording(&mut a.editor),
        );
        if !recording && has_recording {
            c.add("Macro: Playback", "Ctrl+Shift+P", |a, _| {
                a.play_macro(MacroSource::Recorded, Repeat::Times(1))
            });
            c.add("Macro: Save Current Recorded Macro…", "", |a, _| {
                a.macros.open_save_dialog()
            });
        }
        if !recording && (has_recording || !self.macros.saved.is_empty()) {
            c.add("Macro: Run a Macro Multiple Times…", "", |a, _| {
                a.macros.open_run_dialog()
            });
        }
        if !recording {
            for (i, m) in self.macros.saved.iter().enumerate() {
                let name = m.name.clone();
                c.add(
                    format!("Macro: {name}"),
                    m.shortcut.as_deref().unwrap_or_default(),
                    move |a, _| {
                        // Still the macro it was when the palette opened.
                        if a.macros.saved.get(i).is_some_and(|m| m.name == name) {
                            a.play_macro(MacroSource::Saved(i), Repeat::Times(1));
                        }
                    },
                );
            }
        }

        // --- Compare -------------------------------------------------------
        if editing {
            let mut others: Vec<ochrepad_core::BufferId> = Vec::new();
            for b in self
                .tabs()
                .into_iter()
                .filter_map(|v| self.editor.view(v).map(|v| v.buffer()))
            {
                if b != buffer && !others.contains(&b) {
                    others.push(b);
                }
            }
            for other in others {
                c.add(
                    format!("Compare: Compare With: {}", self.buffer_name(other)),
                    "",
                    move |a, _| {
                        let current = a.buffer_id();
                        if a.editor.buffer(other).is_some() && other != current {
                            a.compare(current, other);
                        }
                    },
                );
            }
            if named {
                c.add("Compare: Compare With Saved Version", "", |a, _| {
                    a.compare_with_saved();
                });
            }
            for (name, shortcut, next) in [
                ("Compare: Next Difference", "F8", true),
                ("Compare: Previous Difference", "Shift+F8", false),
            ] {
                c.add(name, shortcut, move |a, _| {
                    let focused = a.focused_compare();
                    match focused.and_then(|id| a.compares.get_mut(id)) {
                        Some(c) if next => c.next_change(&mut a.editor),
                        Some(c) => c.previous_change(&mut a.editor),
                        None => a.note_text("No comparison has the focus"),
                    }
                });
            }
        }

        // --- View ----------------------------------------------------------
        c.add("View: Full Screen", "F11", |_, ui| {
            super::edit_tools::toggle_full_screen(ui)
        });
        let s = &self.settings;
        c.toggle("View: Always on Top", "", s.always_on_top, |a, _| {
            a.settings.always_on_top = !a.settings.always_on_top
        });
        if editing {
            c.add("View: Markdown Preview", "Ctrl+Shift+M", |a, _| {
                a.open_preview()
            });
            c.add("View: Clone to Other View", "", |a, _| {
                a.clone_to_other_view();
            });
        }
        c.add("View: Terminal", "Ctrl+`", |a, _| a.toggle_terminal());
        c.add("View: New Terminal", "Ctrl+Shift+`", |a, _| {
            a.new_terminal();
        });
        if editing {
            c.add("View: Summary…", "", |a, _| a.show_summary());
        }
        if viewer.is_some() {
            c.add("View: Open as Editable…", "", |a, _| {
                if let Some(id) = a.focused_viewer() {
                    a.ask_open_as_editable(id);
                }
            });
        }
        c.add("View: Next Tab", "Ctrl+Tab", |a, _| a.cycle_tab(true));
        c.add("View: Previous Tab", "Ctrl+Shift+Tab", |a, _| {
            a.cycle_tab(false)
        });
        c.add("View: Move Tab Forward", "Ctrl+Shift+PageDown", |a, _| {
            a.move_tab(true)
        });
        c.add("View: Move Tab Backward", "Ctrl+Shift+PageUp", |a, _| {
            a.move_tab(false)
        });
        if editing {
            c.act("View: Fold All", Action::FoldAll);
            c.act("View: Unfold All", Action::UnfoldAll);
            c.act("View: Fold Current", Action::Fold);
            c.act("View: Unfold Current", Action::Unfold);
            for level in 1..=8 {
                c.act(
                    format!("View: Fold Level: {level}"),
                    Action::FoldLevel(level),
                );
            }
            for level in 1..=8 {
                c.act(
                    format!("View: Unfold Level: {level}"),
                    Action::UnfoldLevel(level),
                );
            }
        }
        c.toggle("View: Show Fold Margin", "", s.fold_margin, |a, _| {
            a.settings.fold_margin = !a.settings.fold_margin
        });
        c.toggle("View: Word Wrap", "", s.word_wrap, |a, _| {
            a.settings.word_wrap = !a.settings.word_wrap
        });
        c.toggle("View: Document Map", "", s.document_map, |a, _| {
            a.settings.document_map = !a.settings.document_map
        });
        c.toggle("View: Document List", "", s.show_document_list, |a, _| {
            a.settings.show_document_list = !a.settings.show_document_list
        });
        c.toggle("View: Function List", "", s.show_function_list, |a, _| {
            a.settings.show_function_list = !a.settings.show_function_list
        });
        c.toggle("View: Change History", "", s.change_history, |a, _| {
            a.settings.change_history = !a.settings.change_history
        });
        c.toggle("View: Problems", "", self.problems.open, |a, _| {
            a.problems.open = !a.problems.open
        });
        c.add("View: Language Servers…", "", |a, _| {
            a.show_language_servers = true
        });
        if let Some(v) = viewer.and_then(|id| self.viewers.get(id)) {
            c.toggle("View: Follow (tail -f)", "", v.state.follow, |a, _| {
                if let Some(v) = a.focused_viewer().and_then(|id| a.viewers.get_mut(id)) {
                    v.state.follow = !v.state.follow;
                }
            });
        } else if named {
            c.toggle(
                "View: Follow (tail -f)",
                "",
                self.is_following(buffer),
                |a, _| {
                    let buffer = a.buffer_id();
                    let on = a.is_following(buffer);
                    a.set_following(buffer, !on);
                },
            );
        }
        c.toggle(
            "View: Show Symbol: Show Space and Tab",
            "",
            s.show_space_tab,
            |a, _| a.settings.show_space_tab = !a.settings.show_space_tab,
        );
        c.toggle(
            "View: Show Symbol: Show End of Line",
            "",
            s.show_eol,
            |a, _| a.settings.show_eol = !a.settings.show_eol,
        );
        c.toggle(
            "View: Show Symbol: Show Non-Printing Characters",
            "",
            s.show_non_printing,
            |a, _| a.settings.show_non_printing = !a.settings.show_non_printing,
        );
        c.toggle(
            "View: Show Symbol: Show All Characters",
            "Ctrl+Shift+8",
            s.show_all_characters(),
            |a, _| {
                let all = a.settings.show_all_characters();
                a.settings.set_show_all_characters(!all);
            },
        );
        c.toggle(
            "View: Show Symbol: Show Indent Guide",
            "",
            s.show_indent_guides,
            |a, _| a.settings.show_indent_guides = !a.settings.show_indent_guides,
        );
        c.toggle(
            "View: Show Symbol: Show Wrap Symbol",
            "",
            s.show_wrap_symbol,
            |a, _| a.settings.show_wrap_symbol = !a.settings.show_wrap_symbol,
        );
        c.add("View: Zoom: Zoom In", "Ctrl++", |a, _| a.zoom_by(1));
        c.add("View: Zoom: Zoom Out", "Ctrl+-", |a, _| a.zoom_by(-1));
        c.add("View: Zoom: Restore Default Zoom", "Ctrl+0", |a, _| {
            a.settings.zoom = 100
        });
        c.toggle("View: Sidebar", "Ctrl+Shift+E", s.show_sidebar, |a, _| {
            a.settings.show_sidebar = !a.settings.show_sidebar
        });
        if self.tree.is_some() && named {
            c.add("View: Reveal Active File in Sidebar", "", |a, _| {
                a.reveal_active_file()
            });
        }
        for (name, theme) in [
            ("View: System theme", ThemeChoice::System),
            ("View: Dark mode", ThemeChoice::Dark),
            ("View: Light mode", ThemeChoice::Light),
        ] {
            c.toggle(name, "", s.theme == theme, move |a, _| {
                a.settings.theme = theme
            });
        }

        // --- Settings and ? ------------------------------------------------
        c.add("Settings: Preferences…", "", |a, _| {
            a.show_preferences = true
        });
        c.add("Settings: Edit Keymap…", "", |a, _| a.edit_keymap());
        c.toggle(
            "Settings: Periodic backup and session snapshot",
            "",
            s.backups,
            |a, _| a.settings.backups = !a.settings.backups,
        );
        c.toggle(
            "Settings: Save files automatically",
            "",
            s.write_through,
            |a, _| a.settings.write_through = !a.settings.write_through,
        );
        c.add("?: About", "", |a, _| {
            a.about = Some(super::about::AboutDialog::default())
        });

        // --- Long lists, after everything else -----------------------------
        c.toggle(
            "View: Syntax theme: Automatic (follows dark/light)",
            "",
            s.syntax_theme.is_none(),
            |a, _| a.settings.syntax_theme = None,
        );
        for name in ochrepad_syntax::Registry::theme_names() {
            c.toggle(
                format!("View: Syntax theme: {name}"),
                "",
                s.syntax_theme.as_deref() == Some(name),
                move |a, _| a.settings.syntax_theme = Some(name.to_owned()),
            );
        }
        if editing {
            let current = self.highlighting.language(buffer);
            for name in self.highlighting.languages() {
                let language = name.clone();
                c.toggle(
                    format!("Language: {name}"),
                    "",
                    name == current,
                    move |a, _| a.set_language(&language),
                );
            }
        }
        c.out
    }

    /// Find Next or Previous with the find panel's pattern.
    fn palette_find(&mut self, backward: bool) {
        if self.find.pattern.is_empty() {
            self.note_text("Nothing to find yet (Search: Find…)");
            return;
        }
        match self.focused_viewer() {
            Some(id) => self.viewer_find(id, backward),
            None => {
                self.find.find(&mut self.editor, self.active, backward);
                self.view_state_mut().reveal_caret();
            }
        }
    }
}

/// The list's rows; the command of a row that was clicked.
fn rows(ui: &mut Ui, o: &mut Open) -> Option<usize> {
    let font = FontSelection::from(TextStyle::Body).resolve(ui.style());
    let row_h = ui.text_style_height(&TextStyle::Body) + 8.0;
    let view_h = row_h * ROWS.min(o.shown.len()) as f32;
    if std::mem::take(&mut o.reveal) {
        let top = o.selected as f32 * row_h;
        o.scroll = o.scroll.clamp((top + row_h - view_h).max(0.0), top);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let visuals = ui.visuals().clone();
    let accent = crate::theme::accent(&visuals);
    let mut clicked = None;
    let out = ScrollArea::vertical()
        .max_height(view_h)
        .vertical_scroll_offset(o.scroll)
        .show_rows(ui, row_h, o.shown.len(), |ui, range| {
            for row in range {
                let shown = &o.shown[row];
                let command = &o.commands[shown.command];
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                response.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, true, command.name.as_str())
                });
                if response.clicked() {
                    clicked = Some(shown.command);
                }
                let selected = row == o.selected;
                if selected {
                    ui.painter()
                        .rect_filled(rect, 3.0, visuals.selection.bg_fill);
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
                }
                let color = if selected {
                    visuals.strong_text_color()
                } else {
                    visuals.text_color()
                };
                // The name, with the typed characters picked out.
                let mut job = LayoutJob::default();
                let plain = TextFormat::simple(font.clone(), color);
                let hit = TextFormat::simple(font.clone(), accent);
                let mut matched = shown.matched.iter().copied().peekable();
                for (i, ch) in command.name.chars().enumerate() {
                    let is_hit = matched.next_if_eq(&i).is_some();
                    let format = if is_hit && !selected { &hit } else { &plain };
                    let mut buf = [0u8; 4];
                    job.append(ch.encode_utf8(&mut buf), 0.0, format.clone());
                }
                match command.on {
                    Some(true) => job.append(
                        "  on",
                        0.0,
                        TextFormat::simple(font.clone(), visuals.weak_text_color()),
                    ),
                    Some(false) => job.append(
                        "  off",
                        0.0,
                        TextFormat::simple(font.clone(), visuals.weak_text_color()),
                    ),
                    None => {}
                }
                let inner = rect.shrink2(vec2(8.0, 0.0));
                let painter = ui.painter().with_clip_rect(inner);
                let keys = painter.layout_no_wrap(
                    command.shortcut.clone(),
                    font.clone(),
                    visuals.weak_text_color(),
                );
                let keys_w = keys.size().x;
                painter.galley(
                    pos2(
                        inner.right() - keys_w,
                        inner.center().y - keys.size().y / 2.0,
                    ),
                    keys,
                    color,
                );
                // The name stops short of the keys.
                let name_clip = Rect::from_min_max(
                    inner.min,
                    pos2(inner.right() - keys_w - 12.0, inner.bottom()),
                );
                let name = ui.fonts_mut(|f| f.layout_job(job));
                ui.painter().with_clip_rect(name_clip).galley(
                    pos2(inner.left(), inner.center().y - name.size().y / 2.0),
                    name,
                    color,
                );
            }
        });
    o.scroll = out.state.offset.y;
    clicked
}
