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
        use crate::async_util::perform_async_work;
        use rfd::FileHandle;

        let messages = async move |destination: FileHandle| {
            let result = Self::export_png_to_path(destination.path().to_path_buf()).await;
            if let Err(e) = result {
                log::error!("Failed to export PNG: {:?}", e);
            }
            vec![crate::Message::AsyncDone(AsyncJob::ExportPng)]
        };

        if let Some(path) = path {
            let sender = self.channels.msg_sender.clone();
            perform_async_work(async move {
                for message in messages(path.into()).await {
                    sender.send(message).unwrap();
                }
            });
        } else {
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

    async fn export_png_to_path(path: PathBuf) -> Result<(), ExportError> {
        // 1. Create an image buffer (e.g., Skia surface)
        let size = Vec2::new(1280.0, 720.0);
        let mut surface = create_surface((size.x as i32, size.y as i32));
        surface.canvas().clear(egui_skia_renderer::Color::BLACK);

        // 2. Render the waveform view to the surface
        draw_onto_surface(
            &mut surface,
            |ctx| {
                ctx.memory_mut(|mem| mem.options.tessellation_options.feathering = false);
                // Note: We can't access self here, so we'll need to pass the visuals and draw function
                // For now, this is a placeholder implementation
                ctx.set_visuals(egui::Visuals::dark());
                setup_custom_font(ctx);
                // TODO: Need to pass the draw function or render context
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
