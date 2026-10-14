//! Headless editing core: text, positions, selections, actions, transactions
//! and undo. No egui and no filesystem access (PLAN 3.1, 7).

pub mod action;
pub mod brackets;
pub mod buffer;
pub mod changeset;
pub mod diff;
pub mod display;
pub mod editor;
pub mod fold;
pub mod history;
pub mod indent;
pub mod macros;
pub mod position;
mod resolve;
pub mod search;
pub mod selection;
pub mod text;
pub mod text_tools;
pub mod transform;
pub mod view;

pub use action::{Action, ActionError, Direction, Motion};
pub use buffer::{Buffer, BufferId, CommentSyntax, FileMeta, Indent, LineEdit, LineEnding};
pub use changeset::{Bias, ChangeSet, Edit};
pub use display::{DisplayMap, Padding, RowPos};
pub use editor::{Clock, Editor, ManualClock, Settings, SystemClock};
pub use fold::{FoldSet, FoldStrategy};
pub use history::{EditKind, SavedHistory, Transaction, sha256_hex};
pub use macros::{MacroError, NamedMacro, Repeat};
pub use position::{ByteOffset, CharIdx, LineCol, VisualCol};
pub use search::{SearchError, SearchOptions, SearchQuery, Searcher};
pub use selection::{Rectangle, Selection, SelectionSet};
pub use text::Document;
pub use text_tools::Codec;
pub use transform::{CaseMode, SortOptions};
pub use view::{View, ViewId};
