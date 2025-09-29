use std::path::PathBuf;

use egui_skia_renderer::{create_surface, draw_onto_surface, EncodedImageFormat};
use emath::Vec2;

use crate::{setup_custom_font, SystemState, async_util::AsyncJob, message::ExportFormat};

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
    pub fn export_waveform(&mut self, path: Option<PathBuf>, format: Option<ExportFormat>) {
        let format = format.unwrap_or_default();
        if let Some(path) = path {
            // Simple synchronous export - just like snapshot tests
            if let Err(e) = self.export_to_path(path, format) {
                log::error!("Failed to export waveform: {:?}", e);
            }
        } else {
            // For file dialog, we need to capture the current state for export
            // Since we can't easily serialize the entire SystemState for async export,
            // we'll use a different approach: trigger the export immediately after the dialog
            use rfd::FileHandle;

            let messages = async move |destination: FileHandle| {
                let output_path = destination.path().to_path_buf();
                
                // The actual export needs to happen synchronously with access to SystemState
                // So we'll just return a message that triggers the export
                log::info!("File dialog selected path: {:?}", output_path);
                
                vec![
                    // Trigger the actual export with the selected path
                    crate::Message::ExportWaveform(Some(output_path), Some(format)),
                    crate::Message::AsyncDone(AsyncJob::ExportWaveform),
                ]
            };

            let (title, filter_name, extensions) = match format {
                ExportFormat::Png => (
                    "Export Waveform as PNG",
                    "PNG files (*.png)",
                    vec!["png".to_string()],
                ),
            };
            
            self.file_dialog_save(
                title,
                (filter_name.to_string(), extensions),
                messages,
            );
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn export_waveform(&mut self, path: Option<PathBuf>, format: Option<ExportFormat>) {
        // TODO: Implement WASM-specific export logic
        log::error!("WASM export not yet implemented");
    }

    fn export_to_path(&mut self, path: PathBuf, format: ExportFormat) -> Result<(), ExportError> {
        // 1. Create an image buffer (e.g., Skia surface) - exactly like snapshot tests
        // BUG: fixed size of 1280x720 is not correct and should use the window size or settings overridden size if set
        // TODO: research proper UX for setting plot export size with settings, it may be confusing to have an override size in the settings, have a default size, or follow the window size
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

        // 3. Encode the image buffer to the specified format and write to file
        let image_data = surface
            .image_snapshot()
            .encode(None, self.get_encoded_image_format(format), None)
            .ok_or_else(|| ExportError::RenderError(format!("Failed to encode image to {:?}", format)))?;

        std::fs::write(&path, image_data.as_bytes())?;
        log::info!("Exported waveform as {:?} to {:?}", format, path);
        Ok(())
    }

    fn get_encoded_image_format(&self, format: ExportFormat) -> EncodedImageFormat {
        match format {
            ExportFormat::Png => EncodedImageFormat::PNG,
        }
    }

}
