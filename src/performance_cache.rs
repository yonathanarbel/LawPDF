use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{PageInfo, PageLink, PageTextChar, PdfNavigation, RenderedPage};

const CACHE_VERSION: u32 = 5;
const NAVIGATION_CACHE_VERSION: u32 = 2;
// Do not reuse old entries written for every page: this cache is now reserved
// for renders that are slow enough to benefit from it.
const RENDER_CACHE_VERSION: u32 = 6;
static TEMP_FILE_NONCE: AtomicU64 = AtomicU64::new(1);
static RENDER_WRITES: AtomicU64 = AtomicU64::new(0);
static RENDER_WRITE_QUEUE: OnceLock<Option<crossbeam_channel::Sender<PendingRenderWrite>>> =
    OnceLock::new();

struct PendingRenderWrite {
    path: PathBuf,
    root: Option<PathBuf>,
    page: RenderedPage,
}

fn render_write_queue() -> Option<&'static crossbeam_channel::Sender<PendingRenderWrite>> {
    RENDER_WRITE_QUEUE
        .get_or_init(|| {
            let (sender, receiver) = crossbeam_channel::bounded::<PendingRenderWrite>(2);
            std::thread::Builder::new()
                .name("pdf-render-cache".into())
                .spawn(move || {
                    while let Ok(write) = receiver.recv() {
                        write_rendered_page(&write.path, write.root.as_deref(), &write.page);
                    }
                })
                .ok()?;
            Some(sender)
        })
        .as_ref()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDocumentMetadata {
    pub pages: Vec<PageInfo>,
    pub links: Vec<Vec<PageLink>>,
    pub optimized: bool,
}

#[derive(Debug, Clone)]
pub struct PerformanceCache {
    root: Option<PathBuf>,
}

impl PerformanceCache {
    pub fn new() -> Self {
        Self { root: cache_root() }
    }

    pub fn for_editor(&self) -> Self {
        Self {
            root: self.root.as_ref().map(|root| root.join("editor-v1")),
        }
    }

    #[cfg(test)]
    fn at(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }

    pub fn load_document_metadata(
        &self,
        source: &Path,
        optimized: bool,
    ) -> Option<CachedDocumentMetadata> {
        let key = document_key(source)?;
        let path = self.root.as_ref()?.join("metadata").join(format!(
            "{key}-{}-v{CACHE_VERSION}.json",
            mode_label(optimized)
        ));
        let cached: CachedDocumentMetadata = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        (cached.optimized == optimized).then_some(cached)
    }

    pub fn save_document_metadata(&self, source: &Path, metadata: &CachedDocumentMetadata) {
        let (Some(key), Some(root)) = (document_key(source), &self.root) else {
            return;
        };
        let path = root.join("metadata").join(format!(
            "{key}-{}-v{CACHE_VERSION}.json",
            mode_label(metadata.optimized)
        ));
        if let Ok(bytes) = serde_json::to_vec(metadata) {
            let _ = write_atomic(&path, &bytes);
        }
    }

    pub fn load_page_text(&self, source: &Path, page_index: usize) -> Option<String> {
        fs::read_to_string(self.page_path(source, "text", page_index, "txt")?).ok()
    }

    pub fn save_page_text(&self, source: &Path, page_index: usize, text: &str) {
        if let Some(path) = self.page_path(source, "text", page_index, "txt") {
            let _ = write_atomic(&path, text.as_bytes());
        }
    }

    pub fn load_page_text_chars(
        &self,
        source: &Path,
        page_index: usize,
    ) -> Option<Vec<PageTextChar>> {
        let path = self.page_path(source, "chars-v1", page_index, "bin")?;
        let file = fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take((crate::page_text_codec::MAX_ENCODED_BYTES + 1) as u64)
            .read_to_end(&mut bytes).ok()?;
        crate::page_text_codec::decode(&bytes).ok()
    }

    pub fn save_page_text_chars(&self, source: &Path, page_index: usize, chars: &[PageTextChar]) {
        let Some(path) = self.page_path(source, "chars-v1", page_index, "bin") else {
            return;
        };
        if let Ok(bytes) = crate::page_text_codec::encode(chars) {
            let _ = write_atomic(&path, &bytes);
        }
    }

    pub fn load_document_links(&self, source: &Path) -> Option<Vec<Vec<PageLink>>> {
        let key = document_key(source)?;
        let path = self
            .root
            .as_ref()?
            .join("links")
            .join(format!("{key}-v{CACHE_VERSION}.json"));
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    pub fn load_document_navigation(&self, source: &Path) -> Option<PdfNavigation> {
        let key = document_key(source)?;
        let path = self.root.as_ref()?.join("navigation").join(format!(
            "{key}-nav{NAVIGATION_CACHE_VERSION}-v{CACHE_VERSION}.json"
        ));
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    pub fn save_document_navigation(&self, source: &Path, navigation: &PdfNavigation) {
        let (Some(key), Some(root)) = (document_key(source), &self.root) else {
            return;
        };
        let path = root.join("navigation").join(format!(
            "{key}-nav{NAVIGATION_CACHE_VERSION}-v{CACHE_VERSION}.json"
        ));
        if let Ok(bytes) = serde_json::to_vec(navigation) {
            let _ = write_atomic(&path, &bytes);
        }
    }

    pub fn save_document_links(&self, source: &Path, links: &[Vec<PageLink>]) {
        let (Some(key), Some(root)) = (document_key(source), &self.root) else {
            return;
        };
        let path = root
            .join("links")
            .join(format!("{key}-v{CACHE_VERSION}.json"));
        if let Ok(bytes) = serde_json::to_vec(links) {
            let _ = write_atomic(&path, &bytes);
        }
    }

    #[cfg(test)]
    pub fn load_rendered_page(
        &self,
        source: &Path,
        page_index: usize,
        render_scale: f32,
        fast: bool,
    ) -> Option<RenderedPage> {
        Self::load_rendered_page_at(
            &self.render_path(source, page_index, render_scale, fast)?,
            page_index,
        )
    }

    pub(crate) fn load_rendered_page_at(path: &Path, page_index: usize) -> Option<RenderedPage> {
        let bytes = fs::read(path).ok()?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        let (width, height) = image.dimensions();
        Some(RenderedPage {
            page_index,
            width: width as usize,
            height: height as usize,
            rgba: image.into_raw(),
        })
    }

    #[cfg(test)]
    pub fn save_rendered_page(
        &self,
        source: &Path,
        rendered: &RenderedPage,
        render_scale: f32,
        fast: bool,
    ) {
        let Some(path) = self.render_path(source, rendered.page_index, render_scale, fast) else {
            return;
        };
        write_rendered_page(&path, self.root.as_deref(), rendered);
    }

    /// Queue a slow render without blocking on PNG encoding or directory scans.
    /// The destination was derived before rendering, so a later source edit
    /// cannot put old pixels under a new revision's key.
    pub(crate) fn queue_rendered_page_at(&self, path: PathBuf, rendered: &RenderedPage) -> bool {
        let Some(sender) = render_write_queue() else {
            return false;
        };
        if sender.is_full() {
            return false;
        }
        sender
            .try_send(PendingRenderWrite {
                path,
                root: self.root.clone(),
                page: rendered.clone(),
            })
            .is_ok()
    }

    fn page_path(
        &self,
        source: &Path,
        kind: &str,
        page_index: usize,
        extension: &str,
    ) -> Option<PathBuf> {
        Some(
            self.root
                .as_ref()?
                .join(kind)
                .join(document_key(source)?)
                .join(format!("p{page_index:05}.{extension}")),
        )
    }

    pub(crate) fn render_path(
        &self,
        source: &Path,
        page_index: usize,
        render_scale: f32,
        fast: bool,
    ) -> Option<PathBuf> {
        let scale_key = if render_scale.is_finite() {
            (render_scale.max(0.0) * 1000.0).round() as u32
        } else {
            0
        };
        Some(
            self.root
                .as_ref()?
                .join("render")
                .join(document_key(source)?)
                .join(format!(
                    "{}-p{page_index:05}-s{scale_key:05}-v{RENDER_CACHE_VERSION}.png",
                    if fast { "fast" } else { "crisp" }
                )),
        )
    }
}

fn write_rendered_page(path: &Path, root: Option<&Path>, rendered: &RenderedPage) {
    let mut png = Vec::new();
    let result =
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
            .write_image(
                &rendered.rgba,
                rendered.width as u32,
                rendered.height as u32,
                ExtendedColorType::Rgba8,
            );
    if result.is_ok() && write_atomic(path, &png).is_ok() {
        if let Some(parent) = path.parent() {
            prune_render_directory(parent, 384 * 1024 * 1024);
        }
        if RENDER_WRITES.fetch_add(1, Ordering::Relaxed) % 16 == 0
            && let Some(root) = root
        {
            prune_render_tree(&root.join("render"), 1024 * 1024 * 1024);
        }
    }
}

fn cache_root() -> Option<PathBuf> {
    #[cfg(feature = "devtools")]
    if std::env::var("LAWPDF_DISABLE_PERFORMANCE_CACHE")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
    {
        return None;
    }

    #[cfg(all(feature = "devtools", not(test)))]
    if let Some(path) = std::env::var_os("LAWPDF_BENCH_DATA_DIR") {
        return Some(PathBuf::from(path).join("performance-cache"));
    }

    #[cfg(windows)]
    if !cfg!(test) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return Some(
                PathBuf::from(local)
                    .join("LawPDF")
                    .join("performance-cache"),
            );
        }
    }
    crate::settings::app_data_dir().map(|path| path.join("performance-cache"))
}

fn document_key(source: &Path) -> Option<String> {
    // A print session owns these snapshots; do not retain unsaved edits in the reader cache.
    if source.ancestors().any(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("lawpdf-print-"))
    }) {
        return None;
    }
    let metadata = fs::metadata(source).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let canonical = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(metadata.len().to_le_bytes());
    hasher.update(modified.to_le_bytes());
    let digest = hasher.finalize();
    Some(
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn mode_label(optimized: bool) -> &'static str {
    if optimized { "optimized" } else { "full" }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("cache path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = temporary_path(path);
    fs::write(&temp, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let nonce = TEMP_FILE_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{}.{}.tmp", std::process::id(), nonce));
    PathBuf::from(value)
}

fn prune_render_directory(root: &Path, limit_bytes: u64) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut files = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    entry.path(),
                    metadata.len(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut total = files.iter().map(|entry| entry.1).sum::<u64>();
    if total <= limit_bytes {
        return;
    }
    files.sort_by_key(|entry| entry.2);
    for (path, len, _) in files {
        if total <= limit_bytes.saturating_mul(7) / 8 {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn prune_render_tree(root: &Path, limit_bytes: u64) {
    let mut files = Vec::new();
    collect_render_files(root, &mut files);
    let mut total = files.iter().map(|entry| entry.1).sum::<u64>();
    if total <= limit_bytes {
        return;
    }
    files.sort_by_key(|entry| entry.2);
    for (path, len, _) in files {
        if total <= limit_bytes.saturating_mul(7) / 8 {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn collect_render_files(root: &Path, files: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_render_files(&entry.path(), files);
        } else if metadata.is_file() {
            files.push((
                entry.path(),
                metadata.len(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PdfRect;

    fn test_cache(name: &str) -> (PathBuf, PathBuf, PerformanceCache) {
        let nonce = TEMP_FILE_NONCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lawpdf-cache-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.pdf");
        fs::write(&source, b"cache fingerprint").unwrap();
        let cache = PerformanceCache::at(root.join("cache"));
        (root, source, cache)
    }

    #[test]
    fn text_and_character_geometry_round_trip() {
        let (root, source, cache) = test_cache("text");
        let chars = vec![PageTextChar {
            ch: 'A',
            rect: Some(PdfRect::new(1.0, 2.0, 3.0, 4.0)),
            font_size: Some(12.0),
            bold: true,
            italic: false,
        }];
        cache.save_page_text(&source, 2, "Alpha");
        cache.save_page_text_chars(&source, 2, &chars);
        let links = vec![vec![PageLink {
            rect: PdfRect::new(5.0, 6.0, 7.0, 8.0),
            url: "https://example.com".to_owned(),
            destination_page: None,
            destination_fraction: None,
        }]];
        cache.save_document_links(&source, &links);

        assert_eq!(cache.load_page_text(&source, 2).as_deref(), Some("Alpha"));
        let restored = cache.load_page_text_chars(&source, 2).unwrap();
        assert_eq!(restored[0].ch, 'A');
        assert_eq!(restored[0].rect, chars[0].rect);
        assert_eq!(cache.load_document_links(&source), Some(links));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_character_cache_is_a_miss_and_legacy_json_is_not_reused() {
        let (root, source, cache) = test_cache("characters-version");
        let old = cache.page_path(&source, "chars", 0, "json").unwrap();
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"[]").unwrap();
        assert!(cache.load_page_text_chars(&source, 0).is_none());

        let new = cache.page_path(&source, "chars-v1", 0, "bin").unwrap();
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&new, b"LPC1\x01\0\0\0").unwrap();
        assert!(cache.load_page_text_chars(&source, 0).is_none());

        let chars = [PageTextChar {
            ch: '§', rect: None, font_size: Some(9.5), bold: false, italic: true,
        }];
        cache.save_page_text_chars(&source, 0, &chars);
        let decoded = cache.load_page_text_chars(&source, 0).unwrap();
        assert_eq!((decoded[0].ch, decoded[0].font_size, decoded[0].italic), ('§', Some(9.5), true));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn navigation_cache_rejects_page_only_revision_and_round_trips_positions() {
        let (root, source, cache) = test_cache("navigation-positions");
        let dir = root.join("cache").join("navigation");
        fs::create_dir_all(&dir).unwrap();
        let old_path = dir.join(format!(
            "{}-v{CACHE_VERSION}.json",
            document_key(&source).unwrap()
        ));
        let old_json = br#"{"links":[[{"rect":{"left":0.0,"bottom":0.0,"right":1.0,"top":1.0},"url":"","destination_page":1}]],"outline":[]}"#;
        let old: PdfNavigation = serde_json::from_slice(old_json).unwrap();
        assert_eq!(old.links[0][0].destination_fraction, None);
        fs::write(&old_path, old_json).unwrap();
        assert!(cache.load_document_navigation(&source).is_none());
        let mut updated = old;
        updated.links[0][0].destination_fraction = Some(0.25);
        cache.save_document_navigation(&source, &updated);
        assert_eq!(
            cache.load_document_navigation(&source).unwrap().links[0][0].destination_fraction,
            Some(0.25)
        );
        fs::remove_file(&old_path).unwrap();
        let new_path = dir.join(format!(
            "{}-nav{NAVIGATION_CACHE_VERSION}-v{CACHE_VERSION}.json",
            document_key(&source).unwrap()
        ));
        fs::remove_file(new_path).unwrap();
        fs::remove_dir(dir).unwrap();
        fs::remove_dir(root.join("cache")).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn rendered_page_round_trips_through_png_cache() {
        let (root, source, cache) = test_cache("render");
        let page = RenderedPage {
            page_index: 1,
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 128],
        };
        cache.save_rendered_page(&source, &page, 1.25, true);

        let restored = cache.load_rendered_page(&source, 1, 1.25, true).unwrap();
        assert_eq!(restored.rgba, page.rgba);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_render_write_keeps_its_original_source_revision() {
        let (root, source, cache) = test_cache("async-render");
        let page = RenderedPage {
            page_index: 0,
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let old_path = cache.render_path(&source, 0, 2.0, false).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !cache.queue_rendered_page_at(old_path.clone(), &page) {
            assert!(
                std::time::Instant::now() < deadline,
                "render cache writer remained unavailable"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        fs::write(&source, b"a different and longer PDF revision").unwrap();
        let new_path = cache.render_path(&source, 0, 2.0, false).unwrap();
        assert_ne!(old_path, new_path);
        while !old_path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            PerformanceCache::load_rendered_page_at(&old_path, 0)
                .unwrap()
                .rgba,
            page.rgba
        );
        assert!(PerformanceCache::load_rendered_page_at(&new_path, 0).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
