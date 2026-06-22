use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Cursor,
    path::Path,
};

use helix_view::graphics::Rect;
use thiserror::Error;

pub const CELL_PIXEL_WIDTH: u32 = 16;
pub const CELL_PIXEL_HEIGHT: u32 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePreview {
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub payload_hash: u64,
    pub png: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ImagePreviewError {
    #[error("image decode failed: {0}")]
    Decode(#[from] image::ImageError),
    #[error("preview area is empty")]
    EmptyArea,
}

pub fn is_supported_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        })
}

pub fn decode_image_preview(
    _path: &Path,
    bytes: &[u8],
    cell_area: Rect,
) -> Result<ImagePreview, ImagePreviewError> {
    let max_width = u32::from(cell_area.width) * CELL_PIXEL_WIDTH;
    let max_height = u32::from(cell_area.height) * CELL_PIXEL_HEIGHT;
    if max_width == 0 || max_height == 0 {
        return Err(ImagePreviewError::EmptyArea);
    }

    let image = image::load_from_memory(bytes)?;
    let original_width = image.width();
    let original_height = image.height();
    let scaled = image.thumbnail(max_width, max_height);
    let width = scaled.width();
    let height = scaled.height();

    let mut png = Cursor::new(Vec::new());
    scaled.write_to(&mut png, image::ImageFormat::Png)?;
    let png = png.into_inner();
    let mut hasher = DefaultHasher::new();
    png.hash(&mut hasher);
    let payload_hash = hasher.finish();

    Ok(ImagePreview {
        original_width,
        original_height,
        width,
        height,
        payload_hash,
        png,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::graphics::Rect;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([20, 40, 60, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("test PNG should encode");
        bytes.into_inner()
    }

    #[test]
    fn recognizes_supported_image_paths() {
        assert!(is_supported_image_path("sprite.PNG".as_ref()));
        assert!(is_supported_image_path("photo.jpeg".as_ref()));
        assert!(is_supported_image_path("anim.gif".as_ref()));
        assert!(is_supported_image_path("texture.webp".as_ref()));
        assert!(!is_supported_image_path("main.rs".as_ref()));
    }

    #[test]
    fn rejects_undecodable_supported_image() {
        let error = decode_image_preview(
            "broken.png".as_ref(),
            b"not really an image",
            Rect::new(0, 0, 20, 10),
        )
        .unwrap_err();

        assert!(matches!(error, ImagePreviewError::Decode(_)));
    }

    #[test]
    fn decodes_and_scales_to_fit_cell_area() {
        let preview = decode_image_preview(
            "wide.png".as_ref(),
            &png_bytes(400, 200),
            Rect::new(0, 0, 10, 10),
        )
        .expect("valid PNG should decode");

        assert_eq!(preview.original_width, 400);
        assert_eq!(preview.original_height, 200);
        assert!(preview.width <= 10 * CELL_PIXEL_WIDTH);
        assert!(preview.height <= 10 * CELL_PIXEL_HEIGHT);
        assert_eq!(preview.width / preview.height, 2);
        assert!(!preview.png.is_empty());
        assert_ne!(preview.payload_hash, 0);
    }
}
