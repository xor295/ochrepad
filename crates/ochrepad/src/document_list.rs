//! View > Document List, as in Notepad++: a panel listing the open
//! documents in tab order, to see and switch between many files. The active
//! document's row is highlighted; a click on a row activates its tab and a
//! middle click closes it, which the app does as for the tab itself.

use std::path::Path;
use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, FontId, Galley, Layout, Pos2, Response, ScrollArea, Sense, Stroke, Ui, Vec2, WidgetInfo,
    WidgetType,
};

/// Height of one row, as in the sidebar's tree.
const ROW_H: f32 = 22.0;

/// One open document, as its row shows it.
pub struct Row<'a> {
    /// The title its tab shows: the file name or "new N", with the same
    /// marks for unsaved changes and read-only.
    pub title: String,
    /// Its file: the folder shows dimmed after the title, and the whole
    /// path in a tooltip.
    pub path: Option<&'a Path>,
    pub active: bool,
}

/// What a click in the panel asks for.
pub enum ListRequest {
    /// Activate this row's tab.
    Activate(usize),
    /// Close this row's tab (a middle click).
    Close(usize),
    /// Hide the panel.
    Hide,
}

#[derive(Default)]
pub struct DocumentList {
    /// The active row when the list was last drawn: when another one
    /// becomes active (from the tab bar or the keyboard), it is scrolled
    /// into view.
    active: Option<usize>,
    scroll_offset: f32,
}

impl DocumentList {
    /// The panel's contents: a header, then one row per document.
    pub fn ui(&mut self, ui: &mut Ui, rows: &[Row<'_>]) -> Option<ListRequest> {
        let mut request = None;
        // As tall as the tab bar beside it, as the sidebar's header is.
        let header = ui
            .allocate_ui_with_layout(
                Vec2::new(ui.available_width(), crate::theme::BAR_H),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), crate::theme::BAR_H));
                    ui.add_space(crate::theme::space::M);
                    ui.strong("Document List");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(crate::theme::space::S);
                        if ui
                            .small_button("×")
                            .on_hover_text("Close the Document List")
                            .clicked()
                        {
                            request = Some(ListRequest::Hide);
                        }
                    });
                },
            )
            .response;
        let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        ui.painter().hline(
            header.rect.x_range(),
            header.rect.bottom() - 0.5,
            Stroke::new(1.0, border),
        );
        ui.add_space(crate::theme::space::XS);

        let active = rows.iter().position(|r| r.active);
        let mut scroll = ScrollArea::vertical().auto_shrink(false);
        if active != self.active {
            self.active = active;
            if let Some(i) = active {
                let y = i as f32 * ROW_H;
                let height = ui.available_height();
                if y < self.scroll_offset {
                    scroll = scroll.vertical_scroll_offset(y);
                } else if y + ROW_H > self.scroll_offset + height {
                    scroll = scroll.vertical_scroll_offset(y + ROW_H - height);
                }
            }
        }
        // Rows touch, and `show_rows` counts the spacing between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let output = scroll.show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for i in range {
                let response = row_ui(ui, &rows[i], &font);
                if response.clicked() {
                    request = Some(ListRequest::Activate(i));
                } else if response.middle_clicked() {
                    request = Some(ListRequest::Close(i));
                }
            }
        });
        self.scroll_offset = output.state.offset.y;
        request
    }
}

/// One document's row: its title (cut short with "…" if it does not fit),
/// then its folder, dimmed.
fn row_ui(ui: &mut Ui, row: &Row<'_>, font: &FontId) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, true, row.active, &row.title)
    });
    let painter = ui.painter();
    let v = ui.visuals();
    if row.active {
        painter.rect_filled(rect, 3.0, v.selection.bg_fill);
    } else if response.hovered() {
        painter.rect_filled(
            rect,
            3.0,
            v.widgets.hovered.weak_bg_fill.gamma_multiply(0.6),
        );
    }
    let x = rect.min.x + crate::theme::space::M;
    let width = rect.max.x - crate::theme::space::S - x;
    let color = if row.active {
        v.selection.stroke.color
    } else {
        v.text_color()
    };
    let mut job = LayoutJob::simple_singleline(row.title.clone(), font.clone(), color);
    job.wrap = TextWrapping::truncate_at_width(width);
    let title = painter.layout_job(job);
    let gap = crate::theme::space::S;
    let folder = row
        .path
        .and_then(Path::parent)
        .filter(|_| !title.elided)
        .and_then(|dir| {
            let dir = dir.display().to_string();
            fit_folder(ui, &dir, font, width - title.size().x - gap)
        });
    let cy = rect.center().y;
    if let Some(folder) = folder {
        let at = Pos2::new(x + title.size().x + gap, cy - folder.size().y / 2.0);
        painter.galley(at, folder, color);
    }
    painter.galley(Pos2::new(x, cy - title.size().y / 2.0), title, color);
    match row.path {
        Some(path) => response.on_hover_text(path.display().to_string()),
        None => response,
    }
}

/// `folder` laid out in `width`: whole, or cut from the front at a
/// separator ("…\src\app") so that its end shows, or `None` when not even
/// its last part fits.
fn fit_folder(ui: &Ui, folder: &str, font: &FontId, width: f32) -> Option<Arc<Galley>> {
    let color = ui.visuals().weak_text_color();
    let cuts = folder
        .match_indices(['/', '\\'])
        .map(|(i, _)| i)
        .filter(|&i| i > 0);
    std::iter::once(folder.to_owned())
        .chain(cuts.map(|i| format!("…{}", &folder[i..])))
        .map(|text| ui.painter().layout_no_wrap(text, font.clone(), color))
        .find(|galley| galley.size().x <= width)
}
