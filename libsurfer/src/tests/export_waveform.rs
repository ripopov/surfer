use std::env;

use image::{DynamicImage, GenericImageView};
use project_root::get_project_root;
use test_log::test;

#[allow(unused_imports)]
use crate::{wave_container::VariableRefExt, Message, StartupParams, SystemState, WaveSource};

/// Helper function to validate exported PNG files
fn validate_exported_png(path: &std::path::Path) -> Result<DynamicImage, String> {
    // Check if file exists
    if !path.exists() {
        return Err(format!("Exported PNG file does not exist: {:?}", path));
    }

    // Check file size (should be reasonable for a 1280x720 PNG)
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("Failed to read file metadata: {}", e))?;

    if metadata.len() < 1000 {
        return Err(format!(
            "Exported PNG file is suspiciously small ({} bytes)",
            metadata.len()
        ));
    }

    // Try to load as image to validate it's a proper PNG
    let img =
        image::open(path).map_err(|e| format!("Failed to load exported PNG as image: {}", e))?;

    // Validate dimensions match expected export size
    let (width, height) = img.dimensions();
    if width != 1280 || height != 720 {
        return Err(format!(
            "Exported image has wrong dimensions: {}x{} (expected 1280x720)",
            width, height
        ));
    }

    Ok(img)
}

/// Test that the export menu item is disabled when no waveform data is loaded.
#[test]
fn export_menu_disabled_without_waveform_data() {
    let state = SystemState::new_default_config().unwrap();

    // Verify that no waveform data is loaded
    assert!(
        state.user.waves.is_none(),
        "Expected no waveform data to be loaded"
    );

    // Test the menu enabled state logic directly
    let menu_enabled = state
        .user
        .waves
        .as_ref()
        .map_or(false, |w| w.any_displayed());
    assert!(
        !menu_enabled,
        "Export menu should be disabled when no waveform data is loaded"
    );
}

/// Test that the export menu item is enabled when waveform data is loaded and items are displayed.
#[test]
fn export_menu_enabled_with_waveform_data() {
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

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/counter.vcd")
                    .try_into()
                    .unwrap(),
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

    // Add variables to display
    state.update(Message::AddVariables(vec![
        crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));

    // Wait for signals to load
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    // Verify that waveform data is loaded
    assert!(
        state.user.waves.is_some(),
        "Expected waveform data to be loaded"
    );

    // Test the menu enabled state logic directly
    let menu_enabled = state
        .user
        .waves
        .as_ref()
        .map_or(false, |w| w.any_displayed());
    assert!(
        menu_enabled,
        "Export menu should be enabled when waveform data is loaded and items are displayed"
    );
}

// Export Tests

/// Test that the export function creates a valid PNG file with correct dimensions and content.
#[test]
fn export_waveform_creates_valid_file() {
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
    let output_path = output_dir.join("export_waveform_creates_valid_file.png");

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/counter.vcd")
                    .try_into()
                    .unwrap(),
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

    // Export the waveform as PNG (default format)
    state.export_waveform(Some(output_path.clone()), None);

    // Give it a moment to complete the export
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Process any messages that might have been generated
    state.handle_async_messages();
    state.handle_batch_commands();

    // Validate the exported file
    match validate_exported_png(&output_path) {
        Ok(img) => {
            println!(
                "Successfully exported PNG: {}x{}",
                img.dimensions().0,
                img.dimensions().1
            );

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

/// Test that successful export sets a status message
#[test]
fn export_sets_success_status_message() {
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

    // Create a temporary directory for the export
    let temp_dir = std::env::temp_dir().join("surfer_export_test");
    std::fs::create_dir_all(&temp_dir).expect("Failed to create temp directory");
    let output_path = temp_dir.join("test_export_status.png");

    // Initialize SystemState with waveform data
    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams {
            waves: Some(WaveSource::File(
                get_project_root()
                    .unwrap()
                    .join("examples/counter.vcd")
                    .try_into()
                    .unwrap(),
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

    // Add variables to display
    state.update(Message::AddVariables(vec![
        crate::wave_container::VariableRef::from_hierarchy_string("tb.dut.counter"),
    ]));

    // Wait for signals to load
    crate::tests::snapshot::wait_for_waves_fully_loaded(&mut state, 10);

    // Verify waveform is loaded
    assert!(state.user.waves.is_some(), "Waveform should be loaded");

    // Initially no status message should be set
    assert!(
        state.status_message.is_none(),
        "No status message should be set initially"
    );

    // Export the waveform
    state.export_waveform(Some(output_path.clone()), None);

    // Give it a moment to complete the export
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Verify status message was set
    assert!(
        state.status_message.is_some(),
        "Status message should be set after successful export"
    );
    assert!(
        state.status_message_expiry.is_some(),
        "Status message expiry should be set"
    );

    // Verify the message contains the expected text
    let message = state.status_message.as_ref().unwrap();
    assert!(
        message.contains("Exported to:"),
        "Status message should contain 'Exported to:'"
    );
    assert!(
        message.contains("test_export_status.png"),
        "Status message should contain the filename"
    );

    // Verify the expiry is in the future
    let expiry = state.status_message_expiry.unwrap();
    assert!(
        expiry > std::time::Instant::now(),
        "Status message expiry should be in the future"
    );

    // Clean up
    if output_path.exists() {
        std::fs::remove_file(&output_path).expect("Failed to remove exported PNG file");
    }
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).expect("Failed to remove temp directory");
    }
}
