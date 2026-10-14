//! The ochrepad desktop application (egui).

pub mod app;
pub mod bench;
pub mod compare;
pub mod doc_map;
pub mod editor_view;
pub mod file_tree;
pub mod filter_bar;
pub mod find_panel;
pub mod fonts;
pub mod highlight;
pub mod invisible;
pub mod keymap;
pub mod layout;
pub mod log_rules;
pub mod logo;
pub mod macro_ui;
pub mod preview;
pub mod safe_images;
pub mod search_results;
pub mod settings;
pub mod theme;

pub use app::OchrepadApp;
