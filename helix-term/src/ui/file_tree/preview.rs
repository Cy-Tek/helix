use std::{fs, path::Path, sync::Arc};

use arc_swap::{access::DynAccess, ArcSwap};
use helix_core::syntax;
use helix_view::graphics::Rect;
use helix_view::{editor::Config as EditorConfig, Document};

use crate::ui::picker::{cached_file_preview_from_bytes, CachedPreview, MAX_FILE_SIZE_FOR_PREVIEW};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Document,
    Directory,
    Image,
    UnsupportedImage,
    Binary,
    LargeFile,
    NotFound,
}

pub struct FileTreePreview {
    inner: CachedPreview,
}

impl FileTreePreview {
    pub fn kind(&self) -> PreviewKind {
        match &self.inner {
            CachedPreview::Document(_) => PreviewKind::Document,
            CachedPreview::Directory(_) => PreviewKind::Directory,
            CachedPreview::Image(_) => PreviewKind::Image,
            CachedPreview::UnsupportedImage => PreviewKind::UnsupportedImage,
            CachedPreview::Binary => PreviewKind::Binary,
            CachedPreview::LargeFile => PreviewKind::LargeFile,
            CachedPreview::NotFound => PreviewKind::NotFound,
        }
    }

    pub(crate) fn into_inner(self) -> CachedPreview {
        self.inner
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FileTreePreviewProvider;

impl FileTreePreviewProvider {
    pub fn preview_path(
        &self,
        path: &Path,
        cell_size_pixels: Option<(u16, u16)>,
    ) -> Option<FileTreePreview> {
        self.preview_path_in(path, Rect::new(0, 0, 40, 20), cell_size_pixels)
    }

    pub fn preview_path_in(
        &self,
        path: &Path,
        area: Rect,
        cell_size_pixels: Option<(u16, u16)>,
    ) -> Option<FileTreePreview> {
        let metadata = fs::metadata(path).ok()?;
        if metadata.is_dir() {
            return Some(FileTreePreview {
                inner: CachedPreview::Directory(Vec::new()),
            });
        }

        let bytes = fs::read(path).ok()?;
        cached_file_preview_from_bytes(path, &bytes, metadata.len(), area, cell_size_pixels)
            .map(|inner| FileTreePreview { inner })
    }

    pub fn preview_path_with_loaders(
        &self,
        path: &Path,
        area: Rect,
        cell_size_pixels: Option<(u16, u16)>,
        config: Arc<dyn DynAccess<EditorConfig>>,
        syn_loader: Arc<ArcSwap<syntax::Loader>>,
    ) -> Option<FileTreePreview> {
        let metadata = fs::metadata(path).ok()?;
        if metadata.is_dir() {
            let mut entries = fs::read_dir(path)
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| {
                    let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
                    let mut name = entry.file_name().to_string_lossy().into_owned();
                    if is_dir {
                        name.push('/');
                    }
                    (name, is_dir)
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|(name, is_dir)| (!*is_dir, name.to_ascii_lowercase()));
            return Some(FileTreePreview {
                inner: CachedPreview::Directory(entries),
            });
        }

        if metadata.len() > MAX_FILE_SIZE_FOR_PREVIEW {
            return Some(FileTreePreview {
                inner: CachedPreview::LargeFile,
            });
        }

        let bytes = fs::read(path).ok()?;
        if let Some(inner) =
            cached_file_preview_from_bytes(path, &bytes, metadata.len(), area, cell_size_pixels)
        {
            return Some(FileTreePreview { inner });
        }

        let mut doc = Document::open(path, None, false, config, syn_loader.clone()).ok()?;
        let loader = syn_loader.load();
        if let Some(language_config) = doc.detect_language_config(&loader) {
            doc.language = Some(language_config);
        }
        Some(FileTreePreview {
            inner: CachedPreview::Document(Box::new(doc)),
        })
    }
}
