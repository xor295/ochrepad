//! Headless editing core: text, positions, selections, actions, transactions
//! and undo. No egui and no filesystem access (PLAN 3.1, 7).

pub mod action;
pub mod buffer;
pub mod changeset;
pub mod editor;
pub mod history;
pub mod position;
mod resolve;
pub mod selection;
pub mod text;
pub mod view;

pub use action::{Action, ActionError, Direction, Motion};
pub use buffer::{Buffer, BufferId, FileMeta, LineEdit, LineEnding};
pub use changeset::{Bias, ChangeSet, Edit};
pub use editor::{Clock, Editor, ManualClock, Settings, SystemClock};
pub use history::{EditKind, Transaction};
pub use position::{ByteOffset, CharIdx, LineCol, VisualCol};
pub use selection::{Rectangle, Selection, SelectionSet};
pub use text::Document;
pub use view::{DisplayMap, IdentityDisplayMap, View, ViewId};
