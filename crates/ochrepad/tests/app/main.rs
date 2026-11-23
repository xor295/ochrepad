//! The app's integration tests, one module per feature, in one test
//! binary: each binary links the whole GUI stack, so 46 of them took
//! about a minute to relink after every change and tens of GB of disk.

mod util;

mod basics_ui;
mod bookmarks_ui;
mod change_history_ui;
mod column_ui;
mod compare_ui;
mod completion_ui;
mod doc_map_ui;
mod docs;
mod document_list_ui;
mod document_ui;
mod edit_tools_ui;
mod editing_extras_ui;
mod export_ui;
mod file_menu_ui;
mod find_in_files_ui;
mod find_ui;
mod fold_ui;
mod folder_index;
mod function_list_ui;
mod highlighting;
mod logs_ui;
mod lsp_ui;
mod macro_ui;
mod overtype_ui;
mod palette_ui;
mod polish;
mod preview;
mod real_servers;
mod replace_in_files_ui;
mod robustness_ui;
mod search_all_ui;
mod session;
mod sessions_ui;
mod tabs;
mod tags_ui;
mod terminal;
mod terminal_ui;
mod text_tools_ui;
mod token_styles_ui;
mod transform_ui;
mod view_ui;
mod viewer_ui;
mod visual;
mod widget;
mod workspace_ui;
mod wrap_ui;
