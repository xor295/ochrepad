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
//!
//! They also keep a document from crashing or exhausting the app (the 0.3.0
//! security review): [`LocalFilesOnly`] reads only regular files up to
//! [`MAX_IMAGE_FILE`] (so not `/dev/zero` or a FIFO), [`SafeRasterLoader`]
//! decodes images with size limits and shrinks any larger than the GPU's
//! largest texture, and SVGs are drawn no larger than that either.

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

/// The largest image file the preview reads.
pub const MAX_IMAGE_FILE: u64 = 32 * 1024 * 1024;

/// Whether [`SafeRasterLoader`] takes `uri`: exactly the URIs `egui_extras`'
/// image loader would (no extension, or one of the formats `image` is built
/// to read), so that none reaches it, since it decodes at any size.
fn is_raster(uri: &str) -> bool {
    match Path::new(uri).extension().and_then(|e| e.to_str()) {
        None => true,
        Some(ext) => image::ImageFormat::from_extension(ext.to_lowercase())
            .is_some_and(|f| f.reading_enabled()),
    }
}

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
        .add_image_loader(Arc::new(SafeRasterLoader::default()));
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
/// and drive-less paths are refused, as are paths through a symbolic link
/// or junction that points to one. Elsewhere every path is local.
pub fn is_local_path(path: &Path) -> bool {
    if !cfg!(windows) {
        return true;
    }
    local_text(path) && links_stay_local(path, 0)
}

/// Whether `path` names a drive, judging the text only.
fn local_text(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

/// Whether no folder or file on `path` is a link to a network path. Each
/// prefix is looked at without following it (`symlink_metadata` does not
/// touch the link's target), and a link's target is checked the same way,
/// a few links deep.
fn links_stay_local(path: &Path, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    let mut prefix = PathBuf::new();
    for c in path.components() {
        prefix.push(c);
        let Ok(meta) = std::fs::symlink_metadata(&prefix) else {
            // Missing: nothing further on can be a link.
            return true;
        };
        if meta.file_type().is_symlink() {
            let Ok(target) = std::fs::read_link(&prefix) else {
                return false;
            };
            let target = match prefix.parent() {
                Some(parent) if target.is_relative() => parent.join(target),
                _ => target,
            };
            if !local_text(&target) || !links_stay_local(&target, depth + 1) {
                return false;
            }
        }
    }
    true
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
        if !uri.starts_with(FILE) {
            return Err(LoadError::NotSupported);
        }
        let Some(path) = local_file(uri) else {
            return Err(LoadError::Loading(
                "Images from network paths are not loaded.".to_owned(),
            ));
        };
        // Device files and pipes would be read forever (`metadata` does not
        // open them); a missing file is egui_extras' to report.
        match std::fs::metadata(&path) {
            Ok(m) if !m.is_file() => Err(LoadError::Loading(
                "Only image files are shown, not devices or pipes.".to_owned(),
            )),
            Ok(m) if m.len() > MAX_IMAGE_FILE => Err(LoadError::Loading(format!(
                "Images over {} MB are not shown.",
                MAX_IMAGE_FILE / (1024 * 1024)
            ))),
            _ => Err(LoadError::NotSupported),
        }
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

/// Rasterizes an SVG at `size_hint` (as `egui_extras` does), but no wider or
/// taller than `max_side` pixels: a texture larger than the GPU allows
/// crashes the app, and an SVG can declare any size in a few bytes.
fn render_svg(
    bytes: &[u8],
    size_hint: SizeHint,
    options: &usvg::Options<'_>,
    max_side: usize,
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
    };
    let fit = (max_side as f32 / size.x.max(size.y)).min(1.0);
    let size = (size * fit).round().max(Vec2::splat(1.0));
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
                let max_side = ctx.input(|i| i.max_texture_side);
                let result =
                    render_svg(&bytes, size_hint, &svg_options(dir), max_side).map(Arc::new);
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

/// Decodes a raster image with limits on its size and memory, shrunk to fit
/// in `max_side` pixels each way (keeping its aspect ratio).
fn decode_raster(bytes: &[u8], max_side: usize) -> Result<ColorImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(30_000);
    limits.max_image_height = Some(30_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let source = Vec2::new(image.width() as f32, image.height() as f32);
    let max = max_side.max(1) as u32;
    let image = if image.width() > max || image.height() > max {
        image.thumbnail(max, max)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()).with_source_size(source))
}

/// PNG, JPEG and WebP images for the preview, decoded with
/// [`decode_raster`]'s limits (in place of `egui_extras`' loader, which
/// decodes at any size).
#[derive(Default)]
pub struct SafeRasterLoader {
    cache: Mutex<HashMap<String, Result<Arc<ColorImage>, String>>>,
}

impl SafeRasterLoader {
    pub const ID: &'static str = egui::generate_loader_id!(SafeRasterLoader);
}

impl ImageLoader for SafeRasterLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &Context, uri: &str, _size_hint: SizeHint) -> ImageLoadResult {
        // egui names a frame of a possibly animated image (WebP) `uri#0`;
        // the first frame is drawn, as a still image.
        let uri = egui::decode_animated_image_uri(uri).map_or(uri, |(uri, _frame)| uri);
        if !is_raster(uri) {
            return Err(LoadError::NotSupported);
        }
        if let Some(result) = self.cache.lock().get(uri) {
            return match result.clone() {
                Ok(image) => Ok(ImagePoll::Ready { image }),
                Err(e) => Err(LoadError::Loading(e)),
            };
        }
        match ctx.try_load_bytes(uri)? {
            BytesPoll::Ready { bytes, .. } => {
                let max_side = ctx.input(|i| i.max_texture_side);
                let result = decode_raster(&bytes, max_side).map(Arc::new);
                self.cache.lock().insert(uri.to_owned(), result.clone());
                match result {
                    Ok(image) => Ok(ImagePoll::Ready { image }),
                    Err(e) => Err(LoadError::Loading(e)),
                }
            }
            BytesPoll::Pending { size } => Ok(ImagePoll::Pending { size }),
        }
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .values()
            .map(|r| match r {
                Ok(image) => image.pixels.len() * size_of::<egui::Color32>(),
                Err(e) => e.len(),
            })
            .sum()
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
        let image = render_svg(svg, SizeHint::Scale(1.0.into()), &svg_options(None), 2048).unwrap();
        assert_eq!(image.size, [4, 2]);
    }

    #[test]
    fn huge_svgs_are_drawn_at_most_the_largest_texture() {
        // A few bytes declaring 60000 x 30000 pixels (7 GB as RGBA).
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="60000" height="30000"><rect width="60000" height="30000" fill="red"/></svg>"#;
        let image = render_svg(svg, SizeHint::Scale(1.0.into()), &svg_options(None), 2048).unwrap();
        assert_eq!(image.size, [2048, 1024]);
    }

    #[test]
    fn large_raster_images_are_shrunk_to_fit() {
        let mut png = Vec::new();
        image::RgbaImage::new(9000, 30)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = decode_raster(&png, 8192).unwrap();
        assert!(
            image.size[0] <= 8192 && image.size[1] <= 8192,
            "{:?}",
            image.size
        );
        assert_eq!(image.source_size, Vec2::new(9000.0, 30.0));
        assert!(decode_raster(b"not an image", 8192).is_err());
        // Every format the preview offers decodes. (BMP is not one: only
        // the Windows build has it, because the clipboard crate wants it.)
        let small = image::RgbImage::new(32, 24);
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
        ] {
            let mut bytes = Vec::new();
            small
                .write_to(&mut std::io::Cursor::new(&mut bytes), format)
                .unwrap();
            let decoded = decode_raster(&bytes, 8192);
            assert!(decoded.is_ok(), "{format:?}: {:?}", decoded.err());
        }
    }

    #[test]
    fn devices_pipes_and_huge_files_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let uri = |p: &Path| {
            let s = p.display().to_string().replace('\\', "/");
            if s.starts_with('/') {
                format!("file://{s}")
            } else {
                format!("file:///{s}")
            }
        };
        let ctx = Context::default();
        // A folder is not a file.
        match LocalFilesOnly.load(&ctx, &uri(dir.path())) {
            Err(LoadError::Loading(msg)) => assert!(msg.contains("not devices"), "{msg}"),
            other => panic!("not refused: {:?}", other.is_ok()),
        }
        let big = dir.path().join("big.png");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_IMAGE_FILE + 1)
            .unwrap();
        match LocalFilesOnly.load(&ctx, &uri(&big)) {
            Err(LoadError::Loading(msg)) => assert!(msg.contains("over 32 MB"), "{msg}"),
            other => panic!("not refused: {:?}", other.is_ok()),
        }
        let small = dir.path().join("small.png");
        std::fs::write(&small, b"x").unwrap();
        assert!(matches!(
            LocalFilesOnly.load(&ctx, &uri(&small)),
            Err(LoadError::NotSupported)
        ));
        if cfg!(unix) {
            assert!(LocalFilesOnly.load(&ctx, "file:///dev/zero").is_err());
        }
    }
}
