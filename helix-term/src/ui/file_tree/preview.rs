use std::{fs, path::Path};

use helix_view::graphics::Rect;

use crate::ui::picker::{cached_file_preview_from_bytes, CachedPreview};

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
        let metadata = fs::metadata(path).ok()?;
        if metadata.is_dir() {
            return Some(FileTreePreview {
                inner: CachedPreview::Directory(Vec::new()),
            });
        }

        let bytes = fs::read(path).ok()?;
        let area = Rect::new(0, 0, 40, 20);
        cached_file_preview_from_bytes(path, &bytes, metadata.len(), area, cell_size_pixels)
            .map(|inner| FileTreePreview { inner })
    }
}
