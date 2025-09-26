use std::env;

use crate::wave_container::VariableRefExt;

/// Test that the export function can be called without crashing.
/// This is a basic smoke test since the actual export implementation is still a placeholder.
#[test]
fn export_png_function_callable() {
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

    // Create a simple test that directly calls the export function
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

    // Test that the export function can be called without panicking
    // Note: The current implementation is a placeholder, so we don't expect a file to be created
    state.export_png(Some(output_path.clone()));

    // Give it a moment to start the async operation
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Process any messages that might have been generated
    state.handle_async_messages();
    state.handle_batch_commands();

    // Clean up
    if output_path.exists() {
        std::fs::remove_file(&output_path).expect("Failed to remove exported PNG file");
    }
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir).expect("Failed to remove temp directory");
    }
}

/// Test that the export message is handled correctly by the system.
/// This verifies the message handling path without expecting file creation.
#[test]
fn export_png_message_handling() {
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

    // Create a simple test that uses the message system
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

    // Send export message - this should not panic
    state.update(crate::Message::ExportPng(Some(output_path.clone())));

    // Give it a moment to process the message
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Process any messages that might have been generated
    state.handle_async_messages();
    state.handle_batch_commands();

    // Clean up
    if output_path.exists() {
        std::fs::remove_file(&output_path).expect("Failed to remove exported PNG file");
    }
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir).expect("Failed to remove temp directory");
    }
}
