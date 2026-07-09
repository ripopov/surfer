use std::fs::File;
use std::io::{Cursor, SeekFrom};
use std::path::PathBuf;

use crate::cbor_decoder::CborDecoder;
use crate::ftr_parser;
use crate::ftr_parser::FtrParser;
use crate::types::FTR;

/// The function you probably want to call first.
/// Parses the file with the given name and returns a FTR variable with all streams, generators and relations already accessible.
/// However, it does not yet load the transactions themselves into memory. This can be done with 'load_stream_into_memory()'.
pub fn parse_ftr(file_name: PathBuf) -> Result<FTR, String> {
    let mut ftr = FTR::default();
    let reader = File::open(&file_name).map_err(|e| e.to_string())?;
    ftr.path = Some(file_name);
    let mut ftr_parser = FtrParser::new(&mut ftr);
    ftr_parser.load(reader)?;
    Ok(ftr)
}

pub fn parse_ftr_from_bytes(bytes: Vec<u8>) -> Result<FTR, String> {
    let mut ftr = FTR::default();
    let mut ftr_parser = FtrParser::new(&mut ftr);
    ftr_parser.load(Cursor::new(bytes))?;
    ftr_parser::connect_relations_and_transactions(&mut ftr);
    Ok(ftr)
}

pub fn is_ftr<R: std::io::Read + std::io::Seek>(input: &mut R) -> Result<bool, String> {
    let mut cbor_decoder = CborDecoder::new(input);
    let tag = cbor_decoder.read_tag()?;
    cbor_decoder
        .input_stream
        .seek(SeekFrom::Start(0))
        .map_err(|_| "Cannot seek to beginning of file after file type check".to_string())?;
    Ok(tag == 55799)
}

#[test]
fn parse_test_file() {
    let path = PathBuf::from("./examples/test.ftr");

    if let Ok(mut ftr) = parse_ftr(path) {
        assert!(!ftr.tx_generators.is_empty());
        assert!(!ftr.tx_streams.is_empty());
        assert!(ftr
            .load_stream_into_memory(crate::types::StreamId(1))
            .is_ok());
    } else {
        assert!(false);
    }
}

#[test]
fn parse_test_file_from_bytes() {
    let bytes = std::fs::read("./examples/test.ftr").unwrap();

    let ftr = parse_ftr_from_bytes(bytes);
    assert!(ftr.is_ok());
    assert!(!ftr.as_ref().unwrap().tx_generators.is_empty());
    assert!(!ftr.as_ref().unwrap().tx_streams.is_empty());
}
