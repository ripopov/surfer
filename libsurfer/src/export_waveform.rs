//! Waveform export functionality for PNG and JPEG formats.
//!
//! This module provides the core functionality for exporting waveform views as image files.
//! It supports both interactive GUI-based exports through file dialogs and programmatic
//! exports with direct file paths.
//!
//! # Features
//!
//! - **Format Detection**: Automatic PNG/JPEG format detection from file extensions
//! - **High-Quality Rendering**: Uses Skia for consistent, high-quality image generation
//! - **Error Handling**: Comprehensive error handling with user-friendly messages
//! - **Status Integration**: Integration with the application's status bar for user feedback
//!
//! # Usage
//!
//! The main entry point is through the `SystemState::export_waveform` method, which can be
//! called either with a direct file path or `None` to trigger a file dialog.
//!
//! ```rust
//! // Direct export to a specific path
//! state.export_waveform(Some(PathBuf::from("output.png")), None);
//!
//! // Trigger file dialog for user to choose path
//! state.export_waveform(None, None);
//! ```
//!
//! # Limitations
//!
//! - Currently exports at a fixed 1280x720 resolution
//! - Only supports PNG and JPEG formats
//! - Not available on WASM targets

use std::path::PathBuf;
use serde::Deserialize;

use egui_skia_renderer::{create_surface, draw_onto_surface, EncodedImageFormat};
use emath::Vec2;

use crate::{setup_custom_font, SystemState, async_util::AsyncJob};

/// Supported export formats for plot export
/// 
/// Note: Support for additional image formats (WebP, BMP, GIF, HEIF, AVIF, SVG, etc.)
/// is out of scope for this implementation and can be considered in a future PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ExportFormat {
    Png,
    Jpeg,
}

impl Default for ExportFormat {
    fn default() -> Self {
        ExportFormat::Png
    }
}

impl ExportFormat {
    pub fn file_extension(&self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
        }
    }
    
    pub fn mime_type(&self) -> &'static str {
        match self {
            ExportFormat::Png => "image/png",
            ExportFormat::Jpeg => "image/jpeg",
        }
    }
    
    /// Get a human-readable description of the format
    pub fn description(&self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG (Portable Network Graphics)",
            ExportFormat::Jpeg => "JPEG (Joint Photographic Experts Group)",
        }
    }
}

/// Error types that can occur during waveform export operations.
#[derive(Debug)]
pub enum ExportError {
    /// Error occurred during image rendering or encoding
    RenderError(String),
    /// Error occurred during file I/O operations
    IoError(std::io::Error),
}

impl From<std::io::Error> for ExportError {
    fn from(err: std::io::Error) -> Self {
        ExportError::IoError(err)
    }
}

/// Determine export format from file path extension.
///
/// This function examines the file extension of the provided path and returns the
/// appropriate `ExportFormat`. If the extension is not recognized or the path has
/// no extension, it falls back to the provided default format.
///
/// # Supported Extensions
///
/// - `.png` → `ExportFormat::Png`
/// - `.jpg`, `.jpeg` → `ExportFormat::Jpeg`
///
/// # Parameters
///
/// * `path` - The file path to examine
/// * `default_format` - The format to return if extension is not recognized
///
/// # Returns
///
/// The detected export format or the default format if detection fails.
///
/// # Example
///
/// ```rust
/// use std::path::PathBuf;
/// use libsurfer::message::ExportFormat;
/// use libsurfer::export_waveform::detect_format_from_path;
///
/// let png_path = PathBuf::from("output.png");
/// let format = detect_format_from_path(&png_path, ExportFormat::Jpeg);
/// assert_eq!(format, ExportFormat::Png);
/// ```
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
    /// Export the current waveform view as a PNG or JPEG image.
    ///
    /// This method provides the main entry point for waveform export functionality.
    /// It can be called in two modes:
    ///
    /// 1. **Direct Export**: Provide a file path to export directly to that location
    /// 2. **Interactive Export**: Pass `None` to trigger a file dialog for user path selection
    ///
    /// # Parameters
    ///
    /// * `path` - Optional file path for direct export. If `None`, a file dialog will be shown.
    /// * `default_format` - Optional default format if not detected from path. Defaults to PNG.
    ///
    /// # Behavior
    ///
    /// - **Format Detection**: Automatically detects PNG/JPEG format from file extension
    /// - **Status Feedback**: Displays success message in status bar upon completion
    /// - **Error Handling**: Logs errors and provides user feedback for failures
    /// - **WASM Support**: Not implemented for WASM targets (logs error)
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::PathBuf;
    /// 
    /// // Direct export to PNG
    /// state.export_waveform(Some(PathBuf::from("waveform.png")), None);
    /// 
    /// // Direct export to JPEG with explicit format
    /// state.export_waveform(Some(PathBuf::from("waveform.jpg")), Some(ExportFormat::Jpeg));
    /// 
    /// // Interactive export via file dialog
    /// state.export_waveform(None, None);
    /// ```
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

    /// Export functionality for WASM targets.
    ///
    /// Currently not implemented. Logs an error message indicating that WASM export
    /// support is not yet available.
    #[cfg(target_arch = "wasm32")]
    pub fn export_waveform(&mut self, path: Option<PathBuf>, format: Option<ExportFormat>) {
        // TODO: Implement WASM-specific export logic
        log::error!("WASM export not yet implemented");
    }

    /// Internal method to perform the actual export to a specific file path.
    ///
    /// This method handles the core export logic including:
    /// - Creating a Skia surface for rendering
    /// - Drawing the waveform to the surface
    /// - Encoding the image in the specified format
    /// - Writing the file to disk
    /// - Setting success status message
    ///
    /// # Parameters
    ///
    /// * `path` - The file path where the image should be saved
    /// * `format` - The image format (PNG or JPEG)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success or `Err(ExportError)` on failure.
    ///
    /// # Current Limitations
    ///
    /// - Fixed export size of 1280x720 pixels
    /// - Uses black background color
    /// - No viewport-specific rendering
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

    /// Convert ExportFormat enum to Skia's EncodedImageFormat.
    ///
    /// This is a helper method that maps our internal `ExportFormat` enum to the
    /// corresponding Skia encoding format for image generation.
    ///
    /// # Parameters
    ///
    /// * `format` - The export format (PNG or JPEG)
    ///
    /// # Returns
    ///
    /// The corresponding Skia `EncodedImageFormat` for encoding the image.
    fn get_encoded_image_format(&self, format: ExportFormat) -> EncodedImageFormat {
        match format {
            ExportFormat::Png => EncodedImageFormat::PNG,
            ExportFormat::Jpeg => EncodedImageFormat::JPEG,
        }
    }

}
