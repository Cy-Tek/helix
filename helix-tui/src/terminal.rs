//! Terminal interface provided through the [Terminal] type.
//! Frontend for [Backend]

use crate::{backend::Backend, buffer::Buffer};
use helix_view::editor::{Config as EditorConfig, KittyKeyboardProtocolConfig};
use helix_view::graphics::{CursorKind, Rect};
use std::collections::HashMap;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaImage {
    pub id: u32,
    pub area: Rect,
    pub width: u32,
    pub height: u32,
    pub payload_hash: u64,
    pub png: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaCommand {
    Image(MediaImage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaOperation {
    RenderImage(MediaImage),
    ClearImage { id: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MediaImageKey {
    id: u32,
    /// Placement size in cells. Screen position is intentionally excluded: images are positioned
    /// by Unicode placeholder cells in the text grid, so scrolling moves them without needing the
    /// image data to be re-transmitted.
    cols: u16,
    rows: u16,
    payload_hash: u64,
}

impl From<&MediaImage> for MediaImageKey {
    fn from(image: &MediaImage) -> Self {
        Self {
            id: image.id,
            cols: image.area.width,
            rows: image.area.height,
            payload_hash: image.payload_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// UNSTABLE
enum ResizeBehavior {
    Fixed,
    Auto,
}

#[derive(Debug, Clone, PartialEq)]
/// UNSTABLE
pub struct Viewport {
    area: Rect,
    resize_behavior: ResizeBehavior,
}

/// Terminal configuration
#[derive(Debug)]
pub struct Config {
    pub enable_mouse_capture: bool,
    pub force_enable_extended_underlines: bool,
    pub force_enable_kitty_graphics: bool,
    pub kitty_keyboard_protocol: KittyKeyboardProtocolConfig,
}

impl From<&EditorConfig> for Config {
    fn from(config: &EditorConfig) -> Self {
        Self {
            enable_mouse_capture: config.mouse,
            force_enable_extended_underlines: config.undercurl,
            force_enable_kitty_graphics: config.force_enable_kitty_graphics,
            kitty_keyboard_protocol: config.kitty_keyboard_protocol,
        }
    }
}

impl Viewport {
    /// UNSTABLE
    pub fn fixed(area: Rect) -> Viewport {
        Viewport {
            area,
            resize_behavior: ResizeBehavior::Fixed,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Options to pass to [`Terminal::with_options`]
pub struct TerminalOptions {
    /// Viewport used to draw to the terminal
    pub viewport: Viewport,
}

/// Interface to the terminal backed by crossterm
#[derive(Debug)]
pub struct Terminal<B>
where
    B: Backend,
{
    backend: B,
    /// Holds the results of the current and previous draw calls. The two are compared at the end
    /// of each draw pass to output the necessary updates to the terminal
    buffers: [Buffer; 2],
    /// Index of the current buffer in the previous array
    current: usize,
    /// Kind of cursor (hidden or others)
    cursor_kind: CursorKind,
    /// Viewport
    viewport: Viewport,
    /// Images currently displayed on the terminal, keyed by their image id. Allows multiple
    /// simultaneous images (e.g. several diagrams in a markdown preview) to be diffed against the
    /// next draw so that only changed images are re-transmitted and removed ones are cleared.
    current_media: HashMap<u32, MediaImageKey>,
}

/// Default terminal size: 80 columns, 24 lines
pub const DEFAULT_TERMINAL_SIZE: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

impl<B> Terminal<B>
where
    B: Backend,
{
    /// Wrapper around Terminal initialization. Each buffer is initialized with a blank string and
    /// default colors for the foreground and the background
    pub fn new(backend: B) -> io::Result<Terminal<B>> {
        let size = backend.size().unwrap_or(DEFAULT_TERMINAL_SIZE);
        Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport {
                    area: size,
                    resize_behavior: ResizeBehavior::Auto,
                },
            },
        )
    }

    /// UNSTABLE
    pub fn with_options(backend: B, options: TerminalOptions) -> io::Result<Terminal<B>> {
        Ok(Terminal {
            backend,
            buffers: [
                Buffer::empty(options.viewport.area),
                Buffer::empty(options.viewport.area),
            ],
            current: 0,
            cursor_kind: CursorKind::Block,
            viewport: options.viewport,
            current_media: HashMap::new(),
        })
    }

    pub fn claim(&mut self) -> io::Result<()> {
        self.backend.claim()
    }

    pub fn reconfigure(&mut self, config: Config) -> io::Result<()> {
        self.backend.reconfigure(config)
    }

    pub fn restore(&mut self) -> io::Result<()> {
        self.backend.restore()
    }

    // /// Get a Frame object which provides a consistent view into the terminal state for rendering.
    // pub fn get_frame(&mut self) -> Frame<B> {
    //     Frame {
    //         terminal: self,
    //         cursor_position: None,
    //     }
    // }

    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn supports_kitty_graphics(&self) -> bool {
        self.backend.supports_kitty_graphics()
    }

    pub fn cell_size_pixels(&self) -> Option<(u16, u16)> {
        self.backend.cell_size_pixels()
    }

    /// Diff the requested set of images against the ones currently displayed and emit the minimal
    /// set of backend operations. Images that are new or whose placement/content changed are
    /// re-transmitted; images that are no longer requested are cleared. If two commands share an
    /// id, the last one wins.
    pub fn draw_media(&mut self, commands: &[MediaCommand]) -> io::Result<()> {
        // Look-up of requested ids; if an id appears twice the last command wins (matching the
        // render loop below, which transmits in command order).
        let mut requested: HashMap<u32, &MediaImage> = HashMap::new();
        for command in commands {
            match command {
                MediaCommand::Image(image) => {
                    requested.insert(image.id, image);
                }
            }
        }

        let mut changed = false;

        // Clear images that are no longer requested. Sorted for deterministic output.
        let mut stale: Vec<u32> = self
            .current_media
            .keys()
            .copied()
            .filter(|id| !requested.contains_key(id))
            .collect();
        stale.sort_unstable();
        for id in stale {
            self.backend.clear_image(id)?;
            self.current_media.remove(&id);
            changed = true;
        }

        // Render images that are new or whose key changed, in the order they were requested so
        // that later images stack above earlier ones consistently.
        for command in commands {
            let MediaCommand::Image(image) = command;
            // Skip all but the last command for a duplicated id.
            if !std::ptr::eq(*requested.get(&image.id).unwrap(), image) {
                continue;
            }
            let key = MediaImageKey::from(image);
            if self.current_media.get(&image.id) != Some(&key) {
                // Clear the previous placement for this id before re-transmitting so a moved
                // image does not leave a ghost behind.
                if self.current_media.contains_key(&image.id) {
                    self.backend.clear_image(image.id)?;
                }
                self.backend.render_image(image)?;
                self.current_media.insert(image.id, key);
                changed = true;
            }
        }

        if changed {
            self.backend.flush()?;
        }

        Ok(())
    }

    /// Obtains a difference between the previous and the current buffer and passes it to the
    /// current backend for drawing.
    pub fn flush(&mut self) -> io::Result<()> {
        let previous_buffer = &self.buffers[1 - self.current];
        let current_buffer = &self.buffers[self.current];
        let updates = previous_buffer.diff(current_buffer);
        self.backend.draw(updates.into_iter())
    }

    /// Updates the Terminal so that internal buffers match the requested size. Requested size will
    /// be saved so the size can remain consistent when rendering.
    /// This leads to a full clear of the screen.
    pub fn resize(&mut self, area: Rect) -> io::Result<()> {
        self.buffers[self.current].resize(area);
        self.buffers[1 - self.current].resize(area);
        self.viewport.area = area;
        self.clear()
    }

    /// Queries the backend for size and resizes if it doesn't match the previous size.
    pub fn autoresize(&mut self) -> io::Result<Rect> {
        let size = self.size();
        if size != self.viewport.area {
            self.resize(size)?;
        };
        Ok(size)
    }

    /// Synchronizes terminal size, calls the rendering closure, flushes the current internal state
    /// and prepares for the next draw call.
    pub fn draw(
        &mut self,
        cursor_position: Option<(u16, u16)>,
        cursor_kind: CursorKind,
    ) -> io::Result<()> {
        // // Autoresize - otherwise we get glitches if shrinking or potential desync between widgets
        // // and the terminal (if growing), which may OOB.
        // self.autoresize()?;

        // let mut frame = self.get_frame();
        // f(&mut frame);
        // // We can't change the cursor position right away because we have to flush the frame to
        // // stdout first. But we also can't keep the frame around, since it holds a &mut to
        // // Terminal. Thus, we're taking the important data out of the Frame and dropping it.
        // let cursor_position = frame.cursor_position;

        // Draw to stdout
        self.flush()?;

        if let Some((x, y)) = cursor_position {
            self.set_cursor(x, y)?;
        }

        match cursor_kind {
            CursorKind::Hidden => self.hide_cursor()?,
            kind => self.show_cursor(kind)?,
        }

        // Swap buffers
        self.buffers[1 - self.current].reset();
        self.current = 1 - self.current;

        // Flush
        self.backend.flush()?;
        Ok(())
    }

    #[inline]
    pub fn cursor_kind(&self) -> CursorKind {
        self.cursor_kind
    }

    pub fn hide_cursor(&mut self) -> io::Result<()> {
        self.backend.hide_cursor()?;
        self.cursor_kind = CursorKind::Hidden;
        Ok(())
    }

    pub fn show_cursor(&mut self, kind: CursorKind) -> io::Result<()> {
        self.backend.show_cursor(kind)?;
        self.cursor_kind = kind;
        Ok(())
    }

    pub fn set_cursor(&mut self, x: u16, y: u16) -> io::Result<()> {
        self.backend.set_cursor(x, y)
    }

    /// Clear the terminal and force a full redraw on the next draw call.
    pub fn clear(&mut self) -> io::Result<()> {
        for id in self.current_media.drain().map(|(id, _)| id) {
            self.backend.clear_image(id)?;
        }
        self.backend.clear()?;
        // Reset the back buffer to make sure the next update will redraw everything.
        self.buffers[1 - self.current].reset();
        Ok(())
    }

    /// Queries the real size of the backend.
    pub fn size(&self) -> Rect {
        self.backend.size().unwrap_or(DEFAULT_TERMINAL_SIZE)
    }
}
