use std::path::PathBuf;

use egui_skia_renderer::{create_surface, draw_onto_surface, EncodedImageFormat};
use emath::Vec2;

use crate::{setup_custom_font, SystemState, async_util::AsyncJob};

// Define an error type for export operations
#[derive(Debug)]
pub enum ExportError {
    RenderError(String),
    IoError(std::io::Error),
    // Add other specific errors as needed
}

impl From<std::io::Error> for ExportError {
    fn from(err: std::io::Error) -> Self {
        ExportError::IoError(err)
    }
}

impl SystemState {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_png(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path {
            // Simple synchronous export - just like snapshot tests
            if let Err(e) = self.export_png_to_path(path) {
                log::error!("Failed to export PNG: {:?}", e);
            }
        } else {
            // For file dialog, we need to capture the current state for export
            // Since we can't easily serialize the entire SystemState for async export,
            // we'll use a different approach: trigger the export immediately after the dialog
            use rfd::FileHandle;

            let messages = async move |destination: FileHandle| {
                let output_path = destination.path().to_path_buf();
                
                // The actual PNG export needs to happen synchronously with access to SystemState
                // So we'll just return a message that triggers the export
                log::info!("File dialog selected path: {:?}", output_path);
                
                vec![
                    // Trigger the actual export with the selected path
                    crate::Message::ExportPng(Some(output_path)),
                    crate::Message::AsyncDone(AsyncJob::ExportPng),
                ]
            };

            self.file_dialog_save(
                "Export Plot as PNG",
                (
                    "PNG files (*.png)".to_string(),
                    vec!["png".to_string()],
                ),
                messages,
            );
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn export_png(&mut self, path: Option<PathBuf>) {
        // TODO: Implement WASM-specific export logic
        log::error!("WASM export not yet implemented");
    }

    fn export_png_to_path(&mut self, path: PathBuf) -> Result<(), ExportError> {
        // 1. Create an image buffer (e.g., Skia surface) - exactly like snapshot tests
        let size = Vec2::new(1280.0, 720.0);
        let mut surface = create_surface((size.x as i32, size.y as i32));
        surface.canvas().clear(egui_skia_renderer::Color::BLACK);

        // 2. Render the waveform view to the surface - exactly like snapshot tests
        draw_onto_surface(
            &mut surface,
            |ctx| {
                ctx.memory_mut(|mem| mem.options.tessellation_options.feathering = false);
                ctx.set_visuals(self.get_visuals());
                setup_custom_font(ctx);
                self.draw(ctx, Some(size));
            },
            Some(egui_skia_renderer::RasterizeOptions {
                frames_before_screenshot: 5,
                ..Default::default()
            }),
        );

        // 3. Encode the image buffer to PNG and write to file
        let png_data = surface
            .image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .ok_or_else(|| ExportError::RenderError("Failed to encode image to PNG".to_string()))?;

        std::fs::write(&path, png_data.as_bytes())?;
        log::info!("Exported PNG to {:?}", path);
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn export_png(&mut self, path: Option<PathBuf>) -> Result<(), ExportError> {
        // TODO: Implement WASM-specific export logic
        Err(ExportError::RenderError("WASM export not yet implemented".to_string()))
    }
}
