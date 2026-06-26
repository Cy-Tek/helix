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
    pub area: Rect,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLayout {
    pub area: Rect,
    pub width: u32,
    pub height: u32,
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

pub fn fit_image_to_cell_area(
    original_width: u32,
    original_height: u32,
    cell_area: Rect,
    cell_pixel_width: u32,
    cell_pixel_height: u32,
) -> Result<ImageLayout, ImagePreviewError> {
    if original_width == 0
        || original_height == 0
        || cell_area.width == 0
        || cell_area.height == 0
        || cell_pixel_width == 0
        || cell_pixel_height == 0
    {
        return Err(ImagePreviewError::EmptyArea);
    }

    let mut best = None;
    let mut best_aspect_error = f64::INFINITY;
    let mut best_area = 0;
    let target_aspect = original_width as f64 / original_height as f64;

    for height_cells in 1..=cell_area.height {
        for width_cells in 1..=cell_area.width {
            let width = u32::from(width_cells) * cell_pixel_width;
            let height = u32::from(height_cells) * cell_pixel_height;
            let aspect_error = ((width as f64 / height as f64) - target_aspect).abs();
            let area_pixels = width * height;

            if aspect_error < best_aspect_error
                || (aspect_error == best_aspect_error && area_pixels > best_area)
            {
                best_aspect_error = aspect_error;
                best_area = area_pixels;
                let x = cell_area.x + (cell_area.width - width_cells) / 2;
                let y = cell_area.y + (cell_area.height - height_cells) / 2;
                best = Some(ImageLayout {
                    area: Rect::new(x, y, width_cells, height_cells),
                    width,
                    height,
                });
            }
        }
    }

    best.ok_or(ImagePreviewError::EmptyArea)
}

pub fn decode_image_preview(
    _path: &Path,
    bytes: &[u8],
    cell_area: Rect,
    cell_pixel_width: u32,
    cell_pixel_height: u32,
) -> Result<ImagePreview, ImagePreviewError> {
    let image = image::load_from_memory(bytes)?;
    let original_width = image.width();
    let original_height = image.height();
    let layout = fit_image_to_cell_area(
        original_width,
        original_height,
        cell_area,
        cell_pixel_width,
        cell_pixel_height,
    )?;
    let scaled = image.resize_exact(layout.width, layout.height, image::imageops::FilterType::Lanczos3);

    let mut png = Cursor::new(Vec::new());
    scaled.write_to(&mut png, image::ImageFormat::Png)?;
    let png = png.into_inner();
    let mut hasher = DefaultHasher::new();
    png.hash(&mut hasher);
    let payload_hash = hasher.finish();

    Ok(ImagePreview {
        area: layout.area,
        original_width,
        original_height,
        width: layout.width,
        height: layout.height,
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
            CELL_PIXEL_WIDTH,
            CELL_PIXEL_HEIGHT,
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
            CELL_PIXEL_WIDTH,
            CELL_PIXEL_HEIGHT,
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

    #[test]
    fn fits_largest_cell_area_that_preserves_aspect_ratio() {
        let layout = fit_image_to_cell_area(200, 100, Rect::new(3, 5, 10, 10), 10, 20)
            .expect("non-empty area should fit");

        assert_eq!(layout.area, Rect::new(4, 9, 8, 2));
        assert_eq!(layout.width, 80);
        assert_eq!(layout.height, 40);
    }

    #[test]
    fn decodes_and_upscales_small_images_to_rendered_area() {
        let preview = decode_image_preview(
            "small.png".as_ref(),
            &png_bytes(20, 10),
            Rect::new(3, 5, 10, 10),
            10,
            20,
        )
        .expect("valid PNG should decode");

        assert_eq!(preview.area, Rect::new(4, 9, 8, 2));
        assert_eq!(preview.width, 80);
        assert_eq!(preview.height, 40);
    }
}
