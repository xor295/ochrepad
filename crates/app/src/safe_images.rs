//! Images in the Markdown preview may only come from this machine.
//!
//! `egui_extras`' file loader turns `file://host/share/x.jpg` into the UNC
//! path `\\host\share\x.jpg`, and an SVG's `<image href>` is read by usvg
//! with `std::fs::read`, which also accepts UNC paths. On Windows, opening a
//! UNC path to another machine sends the user's NTLM credentials to it, so a
//! Markdown file could leak them just by being previewed. Two loaders,
//! installed in front of `egui_extras`' ones, prevent that:
//!
//! - [`LocalFilesOnly`] refuses `file://` URIs that do not name a file on a
//!   local drive (judged the way `egui_extras` turns URIs into paths);
//! - [`SafeSvgLoader`] renders SVGs with an `<image href>` resolver that only
//!   reads files inside the SVG's own folder (and `data:` images).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Arc;

use egui::load::{
    BytesLoadResult, BytesLoader, BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError,
    SizeHint,
};
use egui::mutex::Mutex;
use egui::{ColorImage, Context, Ui, UiBuilder, Vec2};
use resvg::usvg;

const FILE: &str = "file://";

/// Installs the guards before the preview first draws. `egui_commonmark`
/// installs `egui_extras`' loaders when it draws, and egui asks the most
/// recently added loaders first, so those go in first: drawing an empty
/// document installs them (they are added only once).
pub fn install(ui: &mut Ui) {
    if ui.ctx().is_loader_installed(LocalFilesOnly::ID) {
        return;
    }
    ui.scope_builder(UiBuilder::new().invisible(), |ui| {
        egui_commonmark::CommonMarkViewer::new().show(
            ui,
            &mut egui_commonmark::CommonMarkCache::default(),
            "",
        );
    });
    ui.ctx().add_bytes_loader(Arc::new(LocalFilesOnly));
    ui.ctx()
        .add_image_loader(Arc::new(SafeSvgLoader::default()));
}

/// The path a `file://` URI names, as `egui_extras`' file loader reads it
/// (no percent-decoding), if it is on this machine. `None` for other URIs
/// and for network paths.
pub fn local_file(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix(FILE)?;
    let path = if cfg!(windows) {
        // `file://host/...` is read as `\\host\...`.
        PathBuf::from(rest.strip_prefix('/')?)
    } else {
        PathBuf::from(rest)
    };
    is_local_path(&path).then_some(path)
}

/// On Windows, only paths on a drive (`C:\...`, `\\?\C:\...`) are local;
/// UNC (`\\host\share`, `\\?\UNC\...`), device (`\\.\...`), NT (`\??\...`)
/// and drive-less paths are refused. Elsewhere every path is local.
pub fn is_local_path(path: &Path) -> bool {
    if !cfg!(windows) {
        return true;
    }
    matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

/// Refuses `file://` URIs that are not on this machine; passes everything
/// else on to the next loader.
pub struct LocalFilesOnly;

impl LocalFilesOnly {
    pub const ID: &'static str = egui::generate_loader_id!(LocalFilesOnly);
}

impl BytesLoader for LocalFilesOnly {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, _ctx: &Context, uri: &str) -> BytesLoadResult {
        if uri.starts_with(FILE) && local_file(uri).is_none() {
            return Err(LoadError::Loading(
                "Images from network paths are not loaded.".to_owned(),
            ));
        }
        Err(LoadError::NotSupported)
    }

    fn forget(&self, _uri: &str) {}

    fn forget_all(&self) {}

    fn byte_size(&self) -> usize {
        0
    }
}

/// Where an SVG at `svg_dir` may read `href` from: a file inside `svg_dir`
/// (after resolving `..` and links), never a network path.
pub fn svg_href_path(href: &str, svg_dir: &Path) -> Option<PathBuf> {
    let path = svg_dir.join(href);
    // Check the text first: canonicalizing a UNC path would already reach
    // the network.
    if !is_local_path(&path) || !is_local_path(svg_dir) {
        return None;
    }
    let path = std::fs::canonicalize(path).ok()?;
    let dir = std::fs::canonicalize(svg_dir).ok()?;
    path.starts_with(&dir).then_some(path)
}

fn svg_options(svg_dir: Option<PathBuf>) -> usvg::Options<'static> {
    let read_file = usvg::ImageHrefResolver::default_string_resolver();
    let dir = svg_dir.clone();
    usvg::Options {
        resources_dir: svg_dir,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(move |href, opts| {
                let path = svg_href_path(href, dir.as_deref()?)?;
                read_file(path.to_str()?, opts)
            }),
        },
        ..usvg::Options::default()
    }
}

/// Rasterizes an SVG at `size_hint` (as `egui_extras` does).
fn render_svg(
    bytes: &[u8],
    size_hint: SizeHint,
    options: &usvg::Options<'_>,
) -> Result<ColorImage, String> {
    let tree = usvg::Tree::from_data(bytes, options).map_err(|e| e.to_string())?;
    let source = Vec2::new(tree.size().width(), tree.size().height());
    let size = match size_hint {
        SizeHint::Size {
            width,
            height,
            maintain_aspect_ratio,
        } => {
            if maintain_aspect_ratio {
                let mut size = source * (width as f32 / source.x);
                if size.y > height as f32 {
                    size *= height as f32 / size.y;
                }
                size
            } else {
                Vec2::new(width as f32, height as f32)
            }
        }
        SizeHint::Height(h) => source * (h as f32 / source.y),
        SizeHint::Width(w) => source * (w as f32 / source.x),
        SizeHint::Scale(scale) => scale.into_inner() * source,
    }
    .round();
    let (w, h) = (size.x as u32, size.y as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| format!("Cannot draw an SVG of size {w}x{h}"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w as f32 / source.x, h as f32 / source.y),
        &mut pixmap.as_mut(),
    );
    Ok(
        ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data())
            .with_source_size(source),
    )
}

struct Entry {
    last_used: u64,
    result: Result<Arc<ColorImage>, String>,
}

#[derive(Default)]
struct SvgState {
    pass_index: u64,
    cache: HashMap<String, HashMap<SizeHint, Entry>>,
}

/// `egui_extras`' SVG loader, with [`svg_href_path`] deciding which files an
/// SVG may embed.
#[derive(Default)]
pub struct SafeSvgLoader {
    state: Mutex<SvgState>,
}

impl SafeSvgLoader {
    pub const ID: &'static str = egui::generate_loader_id!(SafeSvgLoader);
}

impl ImageLoader for SafeSvgLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        if !egui::load::has_extension(uri, "svg") {
            return Err(LoadError::NotSupported);
        }
        let mut state = self.state.lock();
        let pass = state.pass_index;
        let bucket = state.cache.entry(uri.to_owned()).or_default();
        if let Some(entry) = bucket.get_mut(&size_hint) {
            entry.last_used = pass;
            return match entry.result.clone() {
                Ok(image) => Ok(ImagePoll::Ready { image }),
                Err(e) => Err(LoadError::Loading(e)),
            };
        }
        match ctx.try_load_bytes(uri)? {
            BytesPoll::Ready { bytes, .. } => {
                let dir = local_file(uri).and_then(|p| p.parent().map(Path::to_path_buf));
                let result = render_svg(&bytes, size_hint, &svg_options(dir)).map(Arc::new);
                bucket.insert(
                    size_hint,
                    Entry {
                        last_used: pass,
                        result: result.clone(),
                    },
                );
                match result {
                    Ok(image) => Ok(ImagePoll::Ready { image }),
                    Err(e) => Err(LoadError::Loading(e)),
                }
            }
            BytesPoll::Pending { size } => Ok(ImagePoll::Pending { size }),
        }
    }

    fn forget(&self, uri: &str) {
        self.state.lock().cache.remove(uri);
    }

    fn forget_all(&self) {
        self.state.lock().cache.clear();
    }

    fn byte_size(&self) -> usize {
        self.state
            .lock()
            .cache
            .values()
            .flat_map(HashMap::values)
            .map(|e| match &e.result {
                Ok(image) => image.pixels.len() * size_of::<egui::Color32>(),
                Err(e) => e.len(),
            })
            .sum()
    }

    fn end_pass(&self, pass_index: u64) {
        let mut state = self.state.lock();
        state.pass_index = pass_index;
        // Keep only the sizes drawn last frame of SVGs drawn at several sizes.
        state.cache.retain(|_, bucket| {
            if bucket.len() >= 2 {
                bucket.retain(|_, e| pass_index <= e.last_used + 1);
            }
            !bucket.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn network_file_uris_are_refused() {
        for uri in [
            "file://host/share/x.jpg",
            "file://203.0.113.5/s/x.jpg",
            "file://///host/share/x.jpg",
            r"file:///\\host\share\x.jpg",
            r"file:///\\?\UNC\host\share\x.jpg",
            r"file:///\\.\pipe\x",
            r"file:///\??\UNC\host\share\x.jpg",
            "file:///host/share/x.jpg",
            "file://img.png",
        ] {
            assert_eq!(local_file(uri), None, "{uri}");
        }
        assert_eq!(
            local_file("file:///C:/docs/img/a.jpg"),
            Some(PathBuf::from("C:/docs/img/a.jpg"))
        );
        assert!(local_file(r"file:///\\?\C:\docs\a.jpg").is_some());
    }

    /// Runs one frame (without a renderer: textures are dropped).
    #[cfg(windows)]
    fn frame(ctx: &Context, ui: impl FnMut(&mut Ui)) {
        ctx.run_ui(Default::default(), ui).textures_delta.clear();
    }

    #[cfg(windows)]
    #[test]
    fn the_guard_answers_before_the_file_loader() {
        let ctx = Context::default();
        frame(&ctx, install);
        match ctx.try_load_bytes("file://host/share/x.jpg") {
            Err(LoadError::Loading(msg)) => assert!(msg.contains("network")),
            Err(e) => panic!("refused for another reason: {e}"),
            Ok(_) => panic!("not refused"),
        }
        // Local files still reach egui_extras' file loader.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.jpg");
        std::fs::write(&file, b"x").unwrap();
        let uri = format!("file:///{}", file.display()).replace('\\', "/");
        assert!(ctx.try_load_bytes(&uri).is_ok(), "{uri}");
        // Previews drawing later (each installs egui_extras' loaders if
        // missing) leave the guard first.
        frame(&ctx, |ui| {
            egui_commonmark::CommonMarkViewer::new().show(
                ui,
                &mut egui_commonmark::CommonMarkCache::default(),
                "![x](file://host/share/x.jpg)",
            );
        });
        assert!(ctx.try_load_bytes("file://host/share/x.jpg").is_err());
        let loaders = ctx.loaders();
        let last_bytes = loaders.bytes.lock().last().map(|l| l.id().to_owned());
        let last_image = loaders.image.lock().last().map(|l| l.id().to_owned());
        assert_eq!(last_bytes.as_deref(), Some(LocalFilesOnly::ID));
        assert_eq!(last_image.as_deref(), Some(SafeSvgLoader::ID));
    }

    #[test]
    fn svgs_embed_only_files_in_their_folder() {
        let dir = tempfile::tempdir().unwrap();
        let svg_dir = dir.path().join("doc");
        std::fs::create_dir_all(svg_dir.join("img")).unwrap();
        std::fs::write(svg_dir.join("img/a.png"), b"x").unwrap();
        std::fs::write(dir.path().join("outside.png"), b"x").unwrap();
        assert!(svg_href_path("img/a.png", &svg_dir).is_some());
        assert!(svg_href_path("../outside.png", &svg_dir).is_none());
        assert!(
            svg_href_path(&dir.path().join("outside.png").to_string_lossy(), &svg_dir).is_none()
        );
        assert!(svg_href_path("missing.png", &svg_dir).is_none());
        if cfg!(windows) {
            assert!(svg_href_path(r"\\host\share\a.png", &svg_dir).is_none());
            assert!(svg_href_path("//host/share/a.png", &svg_dir).is_none());
            assert!(svg_href_path(r"\\?\UNC\host\share\a.png", &svg_dir).is_none());
        }
    }

    #[test]
    fn svgs_still_render_with_a_data_image() {
        // A 1x1 PNG embedded as a data URL.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="red"/><image width="1" height="1" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg=="/></svg>"#;
        let image = render_svg(svg, SizeHint::Scale(1.0.into()), &svg_options(None)).unwrap();
        assert_eq!(image.size, [4, 2]);
    }
}
