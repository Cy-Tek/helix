//! Rendering of [mermaid](https://mermaid.js.org) diagrams to raster images.
//!
//! The diagram source is written to a temporary `.mmd` file and handed to an external renderer
//! (by default `mmdc`, the official `@mermaid-js/mermaid-cli`). The renderer is expected to write
//! a PNG, whose bytes are returned for scaling/placement by the preview component.

use std::{
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{Hash, Hasher},
    io,
    path::PathBuf,
    process::Command,
};

use helix_view::editor::MarkdownPreviewConfig;

/// Errors that can occur while rendering a mermaid diagram.
#[derive(Debug)]
pub enum MermaidError {
    /// The configured renderer binary could not be found on `PATH`.
    RendererNotFound { command: String },
    /// The renderer ran but exited unsuccessfully.
    RenderFailed { command: String, stderr: String },
    /// The renderer reported success but produced no output file.
    NoOutput { command: String },
    /// An I/O error occurred while preparing the input or reading the output.
    Io(io::Error),
}

impl fmt::Display for MermaidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MermaidError::RendererNotFound { command } => write!(
                f,
                "diagram renderer '{command}' not found — install @mermaid-js/mermaid-cli \
                 (npm install -g @mermaid-js/mermaid-cli) or set editor.markdown-preview.diagram-renderer"
            ),
            MermaidError::RenderFailed { command, stderr } => {
                let detail = stderr.trim();
                if detail.is_empty() {
                    write!(f, "diagram renderer '{command}' failed")
                } else {
                    write!(f, "diagram renderer '{command}' failed: {detail}")
                }
            }
            MermaidError::NoOutput { command } => {
                write!(f, "diagram renderer '{command}' produced no image")
            }
            MermaidError::Io(err) => write!(f, "diagram render I/O error: {err}"),
        }
    }
}

impl std::error::Error for MermaidError {}

impl From<io::Error> for MermaidError {
    fn from(err: io::Error) -> Self {
        MermaidError::Io(err)
    }
}

/// A configured mermaid renderer.
#[derive(Debug, Clone)]
pub struct MermaidRenderer {
    command: String,
    args: Vec<String>,
}

impl MermaidRenderer {
    pub fn from_config(config: &MarkdownPreviewConfig) -> Self {
        Self {
            command: config.diagram_renderer.clone(),
            args: config.diagram_renderer_args.clone(),
        }
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    /// Whether the configured renderer binary is available on `PATH`.
    pub fn is_available(&self) -> bool {
        helix_stdx::env::binary_exists(&self.command)
    }

    /// Render the given mermaid `source` to PNG bytes.
    ///
    /// Temporary files are named after a hash of the source so repeated renders of the same
    /// diagram reuse the same paths (and the OS can keep them warm in cache).
    pub fn render_png(&self, source: &str) -> Result<Vec<u8>, MermaidError> {
        if !self.is_available() {
            return Err(MermaidError::RendererNotFound {
                command: self.command.clone(),
            });
        }

        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        self.command.hash(&mut hasher);
        self.args.hash(&mut hasher);
        let stem = format!("helix-mermaid-{:016x}", hasher.finish());

        let dir = std::env::temp_dir();
        let input = dir.join(format!("{stem}.mmd"));
        let output: PathBuf = dir.join(format!("{stem}.png"));

        std::fs::write(&input, source)?;
        // Best-effort: start from a clean slate so a stale output isn't mistaken for success.
        let _ = std::fs::remove_file(&output);

        let result = self.run(&input, &output);

        // Clean up the input regardless of outcome; keep the output only long enough to read it.
        let _ = std::fs::remove_file(&input);

        let run = result?;
        if !run.status.success() {
            let _ = std::fs::remove_file(&output);
            return Err(MermaidError::RenderFailed {
                command: self.command.clone(),
                stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
            });
        }

        match std::fs::read(&output) {
            Ok(bytes) => {
                let _ = std::fs::remove_file(&output);
                if bytes.is_empty() {
                    Err(MermaidError::NoOutput {
                        command: self.command.clone(),
                    })
                } else {
                    Ok(bytes)
                }
            }
            Err(_) => Err(MermaidError::NoOutput {
                command: self.command.clone(),
            }),
        }
    }

    fn run(
        &self,
        input: &std::path::Path,
        output: &std::path::Path,
    ) -> Result<std::process::Output, MermaidError> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .arg("-i")
            .arg(input)
            .arg("-o")
            .arg(output);

        match command.output() {
            Ok(output) => Ok(output),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(MermaidError::RendererNotFound {
                    command: self.command.clone(),
                })
            }
            Err(err) => Err(MermaidError::Io(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer(command: &str) -> MermaidRenderer {
        MermaidRenderer {
            command: command.to_string(),
            args: Vec::new(),
        }
    }

    #[test]
    fn missing_renderer_reports_not_found() {
        let renderer = renderer("definitely-not-a-real-binary-xyz");
        let err = renderer
            .render_png("graph TD; A-->B;")
            .expect_err("missing renderer should error");
        assert!(matches!(err, MermaidError::RendererNotFound { .. }));
    }

    #[test]
    fn not_found_message_mentions_install_hint() {
        let err = MermaidError::RendererNotFound {
            command: "mmdc".to_string(),
        };
        let message = err.to_string();
        assert!(message.contains("mmdc"));
        assert!(message.contains("mermaid-cli"));
    }

    #[test]
    fn from_config_uses_configured_command() {
        let mut config = MarkdownPreviewConfig::default();
        config.diagram_renderer = "custom-renderer".to_string();
        config.diagram_renderer_args = vec!["-t".to_string(), "dark".to_string()];
        let renderer = MermaidRenderer::from_config(&config);
        assert_eq!(renderer.command(), "custom-renderer");
        assert_eq!(renderer.args, vec!["-t".to_string(), "dark".to_string()]);
    }
}
