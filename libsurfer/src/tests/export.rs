use std::env;

use image::{DynamicImage, GenericImageView};
use image_compare::rgb_hybrid_compare;
use project_root::get_project_root;

use crate::wave_container::VariableRefExt;

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

/// Helper function to set up test state with loaded waves
fn setup_test_state() -> crate::SystemState {
    let mut state = crate::SystemState::new_default_config()
        .unwrap()
        .with_params(crate::StartupParams {
            waves: Some(crate::WaveSource::File(
                project_root::get_project_root().unwrap().join("examples/counter.vcd").try_into().unwrap(),
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
    state.update(crate::Message::AddVariables(vec![
        crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));

    // Wait for signals to load
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);
    
    state
}

/// Test that the export function creates a valid PNG file with correct dimensions and content.
#[test]
fn export_png_creates_valid_file() {
    // Set up Tokio runtime for async operations
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
    let output_path = output_dir.join("test_export.png");
    println!("output_path {}", output_path.display());

    // Set up test state with loaded waves
    let mut state = setup_test_state();

    // Test that the export function creates a valid PNG file
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
            // (which would indicate rendering failed)
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

/// Test that the export message creates a valid PNG file with correct dimensions and content.
#[test]
fn export_png_message_creates_valid_file() {
    // Set up Tokio runtime for async operations
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
    let output_path = output_dir.join("test_message_export.png");
    println!("output_path {}", output_path.display());

    // Set up test state with loaded waves
    let mut state = setup_test_state();

    // Send export message - this should create a valid PNG file
    state.update(crate::Message::ExportPng(Some(output_path.clone())));

    // Give it a moment to process the message and complete the export
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Process any messages that might have been generated
    state.handle_async_messages();
    state.handle_batch_commands();

    // Validate the exported file
    match validate_exported_png(&output_path) {
        Ok(img) => {
            println!("Successfully exported PNG via message: {}x{}", img.dimensions().0, img.dimensions().1);
            
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

/// Test that exports are consistent by comparing against a reference image.
/// This test will fail if the export output changes, helping catch regressions.
#[test]
fn export_png_consistency_test() {
    // Set up Tokio runtime for async operations
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
    let output_path = output_dir.join("test_consistency_export.png");
    println!("output_path {}", output_path.display());

    // Set up test state with loaded waves
    let mut state = setup_test_state();

    // Export the PNG
    state.export_png(Some(output_path.clone()));

    // Give it a moment to complete the export
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Process any messages that might have been generated
    state.handle_async_messages();
    state.handle_batch_commands();

    // Validate the exported file exists and is valid
    let exported_img = match validate_exported_png(&output_path) {
        Ok(img) => img,
        Err(e) => panic!("Export validation failed: {}", e),
    };

    // Get the reference image path
    let root = get_project_root().expect("Failed to get project root");
    let reference_path = root.join("snapshots").join("export_consistency_reference.png");
    
    if reference_path.exists() {
        // Compare against reference image
        let reference_img = image::open(&reference_path)
            .expect("Failed to load reference image");
        
        let result = rgb_hybrid_compare(
            &exported_img.clone().into_rgb8(), 
            &reference_img.clone().into_rgb8()
        ).expect("Image comparison failed");
        
        let threshold_score = 0.95; // More lenient threshold for export tests due to potential non-deterministic rendering
        
        if result.score > threshold_score {
            // Images are different - save diff and fail test
            let diff_img = result.image.to_color_map();
            let diff_path = root.join("snapshots").join("export_consistency_diff.png");
            
            diff_img.save(&diff_path)
                .expect("Failed to save diff image");
            
            panic!(
                "Export consistency test failed. Images differ significantly.\n\
                Score: {} (threshold: {})\n\
                Reference: {:?}\n\
                Exported: {:?}\n\
                Diff: {:?}",
                result.score, threshold_score, reference_path, output_path, diff_path
            );
        } else {
            println!("Export consistency test passed. Score: {}", result.score);
        }
    } else {
        // No reference image exists - save current export as reference
        std::fs::create_dir_all(reference_path.parent().unwrap())
            .expect("Failed to create snapshots directory");
        
        exported_img.save(&reference_path)
            .expect("Failed to save reference image");
        
        panic!(
            "No reference image found. Saved current export as reference: {:?}\n\
            Run the test again to verify consistency.",
            reference_path
        );
    }

    // Clean up
    if output_path.exists() {
        std::fs::remove_file(&output_path).expect("Failed to remove exported PNG file");
    }
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir).expect("Failed to remove temp directory");
    }
}