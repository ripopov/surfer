use std::env;

use image::{DynamicImage, GenericImageView};
use project_root::get_project_root;
use test_log::test;

#[allow(unused_imports)]
use crate::{
    wave_container::VariableRefExt,
    Message, StartupParams, SystemState, WaveSource,
};

/// Helper function to validate exported PNG files
fn validate_exported_png(path: &std::path::Path) -> Result<DynamicImage, String> {
    // Check if file exists
    if !path.exists() {
        return Err(format!("Exported PNG file does not exist: {:?}", path));
    }
    
    // Check file size (should be reasonable for a 1280x720 PNG)
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read file metadata: {}", e))?;
    
    if metadata.len() < 1000 {
        return Err(format!("Exported PNG file is suspiciously small ({} bytes)", metadata.len()));
    }
    
    // Try to load as image to validate it's a proper PNG
    let img = image::open(path)
        .map_err(|e| format!("Failed to load exported PNG as image: {}", e))?;
    
    // Validate dimensions match expected export size
    let (width, height) = img.dimensions();
    if width != 1280 || height != 720 {
        return Err(format!("Exported image has wrong dimensions: {}x{} (expected 1280x720)", width, height));
    }
    
    Ok(img)
}

// Export Tests

/// Test that the export function creates a valid PNG file with correct dimensions and content.
#[test]
fn export_png_creates_valid_file() {
    // https://tokio.rs/tokio/topics/bridging
    // We want to run the gui in the main thread, but some long running tasks like
    // loading VCDs should be done asynchronously. We can't just use std::thread to
    // do that due to wasm support, so we'll start a tokio runtime
    let runtime = tokio::runtime::Builder::new_current_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();

    let _enter = runtime.enter();

    std::thread::spawn(move || {
        runtime.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });
    
    let output_dir = env::temp_dir().join("surfer_test_exports");
    std::fs::create_dir_all(&output_dir).expect("Failed to create output directory");
    let output_path = output_dir.join("export_png_creates_valid_file.png");
    
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root().unwrap().join("examples/counter.vcd").try_into().unwrap(),
            )),
            ..Default::default()
        });

    // Wait for waves to load
    let load_start = std::time::Instant::now();
    loop {
        state.handle_async_messages();
        state.handle_batch_commands();
        if state.waves_fully_loaded() {
            break;
        }
        if load_start.elapsed().as_secs() > 10 {
            panic!("Timeout loading waves");
        }
    }

    // Add variables
    state.update(Message::AddVariables(vec![
        crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));

    // Wait for signals to load
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    
    // Export the PNG
    state.export_png(Some(output_path.clone()));
    
    // Give it a moment to complete the export
    std::thread::sleep(std::time::Duration::from_millis(100));
    
    // Process any messages that might have been generated
    state.handle_async_messages();
    state.handle_batch_commands();
    
    // Validate the exported file
    match validate_exported_png(&output_path) {
        Ok(img) => {
            println!("Successfully exported PNG: {}x{}", img.dimensions().0, img.dimensions().1);
            
            // Additional validation: check that the image is not just a solid color
            let rgb_img = img.to_rgb8();
            let pixels: Vec<_> = rgb_img.pixels().collect();
            let first_pixel = pixels[0];
            let all_same_color = pixels.iter().all(|p| p == &first_pixel);
            
            if all_same_color {
                panic!("Exported image appears to be a solid color, indicating rendering may have failed");
            }
        }
        Err(e) => {
            panic!("Export validation failed: {}", e);
        }
    }
    
    // Clean up
    if output_path.exists() {
        std::fs::remove_file(&output_path).expect("Failed to remove exported PNG file");
    }
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir).expect("Failed to remove temp directory");
    }
}
