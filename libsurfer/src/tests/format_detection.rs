use std::path::PathBuf;
use crate::message::ExportFormat;
use crate::export_waveform::detect_format_from_path;

#[test]
fn test_detect_format_from_path_png() {
    let path = PathBuf::from("test.png");
    let format = detect_format_from_path(&path, ExportFormat::Jpeg);
    assert_eq!(format, ExportFormat::Png);
}

#[test]
fn test_detect_format_from_path_jpg() {
    let path = PathBuf::from("test.jpg");
    let format = detect_format_from_path(&path, ExportFormat::Png);
    assert_eq!(format, ExportFormat::Jpeg);
}

#[test]
fn test_detect_format_from_path_jpeg() {
    let path = PathBuf::from("test.jpeg");
    let format = detect_format_from_path(&path, ExportFormat::Png);
    assert_eq!(format, ExportFormat::Jpeg);
}

#[test]
fn test_detect_format_from_path_case_insensitive() {
    let path = PathBuf::from("test.PNG");
    let format = detect_format_from_path(&path, ExportFormat::Jpeg);
    assert_eq!(format, ExportFormat::Png);
}

#[test]
fn test_detect_format_from_path_no_extension() {
    let path = PathBuf::from("test");
    let format = detect_format_from_path(&path, ExportFormat::Png);
    assert_eq!(format, ExportFormat::Png);
}

#[test]
fn test_detect_format_from_path_unknown_extension() {
    let path = PathBuf::from("test.txt");
    let format = detect_format_from_path(&path, ExportFormat::Jpeg);
    assert_eq!(format, ExportFormat::Jpeg);
}

#[test]
fn test_detect_format_from_path_complex_path() {
    let path = PathBuf::from("/home/user/documents/waveform_export.png");
    let format = detect_format_from_path(&path, ExportFormat::Jpeg);
    assert_eq!(format, ExportFormat::Png);
}
