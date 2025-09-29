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

/// Determine export format from file path extension
/// Falls back to default format if extension is not recognized
/// 
/// Note: Only PNG and JPEG formats are supported. Additional formats are out of scope
/// for this implementation and can be considered in a future PR.
pub fn detect_format_from_path(path: &PathBuf, default_format: ExportFormat) -> ExportFormat {
    if let Some(extension) = path.extension() {
        match extension.to_str().unwrap_or("").to_lowercase().as_str() {
            "png" => ExportFormat::Png,
            "jpg" | "jpeg" => ExportFormat::Jpeg,
            _ => default_format, // Use default if extension not recognized
        }
    } else {
        default_format // No extension, use default
    }
}

impl SystemState {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_waveform(&mut self, path: Option<PathBuf>, default_format: Option<ExportFormat>) {
        let default_format = default_format.unwrap_or(ExportFormat::Png);
        if let Some(path) = path {
            // Detect format from file extension
            let format = detect_format_from_path(&path, default_format);
            log::info!("Detected format {:?} from path: {:?}", format, path);
            
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
                
                // Detect format from the selected path
                let format = detect_format_from_path(&output_path, default_format);
                log::info!("File dialog selected path: {:?}, detected format: {:?}", output_path, format);
                
                vec![
                    // Trigger the actual export with the selected path
                    crate::Message::ExportWaveform(Some(output_path), Some(default_format)),
                    crate::Message::AsyncDone(AsyncJob::ExportWaveform),
                ]
            };

            // Create file dialog with PNG and JPEG filters
            let title = "Export Waveform";
            let filter_name = "Image files";
            let extensions = vec!["png".to_string(), "jpg".to_string(), "jpeg".to_string()];
            
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
        let encoded_format = self.get_encoded_image_format(format);
        let image_data = surface
            .image_snapshot()
            .encode(None, encoded_format, None)
            .ok_or_else(|| {
                ExportError::RenderError(format!(
                    "Failed to encode image to {:?}. This format may not be supported in the current Skia build. Try PNG or JPEG instead.",
                    format
                ))
            })?;

        std::fs::write(&path, image_data.as_bytes())?;
        log::info!("Exported waveform as {:?} to {:?}", format, path);
        
        // Set success status message
        self.set_status_message(format!("Exported to: {}", path.display()), 4);
        
        Ok(())
    }

    fn get_encoded_image_format(&self, format: ExportFormat) -> EncodedImageFormat {
        match format {
            ExportFormat::Png => EncodedImageFormat::PNG,
            ExportFormat::Jpeg => EncodedImageFormat::JPEG,
        }
    }

}
