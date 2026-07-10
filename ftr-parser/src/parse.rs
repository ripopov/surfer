use std::fs::File;
use std::io::{Cursor, SeekFrom};
use std::path::PathBuf;

use crate::cbor_decoder::CborDecoder;
use crate::ftr_parser;
use crate::ftr_parser::FtrParser;
use crate::types::FTR;

/// The function you probably want to call first.
/// Parses the file with the given name and returns an FTR variable with all
/// streams, generators, and seekable transaction/relationship directories.
/// Transaction and relationship bodies are loaded on demand.
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

#[test]
fn block_directory_and_streaming_visitor_match_resident_load() {
    use crate::types::{BlockStatus, GeneratorId, StreamId};
    use std::collections::HashMap;

    let path = PathBuf::from("./examples/test.ftr");
    let mut streamed = parse_ftr(path.clone()).expect("parse block directory");
    let blocks = &streamed.get_stream(StreamId(1)).unwrap().tx_blocks;
    assert!(!blocks.is_empty());
    assert!(blocks.iter().enumerate().all(|(ordinal, block)| {
        block.ordinal == ordinal as u64
            && block.encoded_len > 0
            && block.end_time >= block.start_time
            && block.uncompressed_len.is_some_and(|len| len > 0)
    }));

    let mut streamed_counts = HashMap::<GeneratorId, usize>::new();
    let stream_generators = streamed.get_stream(StreamId(1)).unwrap().generators.clone();
    streamed
        .visit_stream_blocks(StreamId(1), |block, transactions| {
            assert_eq!(block.stream_id, StreamId(1));
            for transaction in transactions {
                if stream_generators.contains(&transaction.get_gen_id()) {
                    *streamed_counts.entry(transaction.get_gen_id()).or_default() += 1;
                }
            }
            Ok(())
        })
        .expect("stream transaction blocks");
    assert!(streamed
        .get_stream(StreamId(1))
        .unwrap()
        .tx_blocks
        .iter()
        .all(|block| block.status == BlockStatus::Loaded));
    assert!(streamed
        .get_stream(StreamId(1))
        .unwrap()
        .generators
        .iter()
        .all(|generator| streamed
            .get_generator(*generator)
            .unwrap()
            .transactions
            .is_empty()));

    let mut resident = parse_ftr(path).expect("parse resident comparison");
    resident
        .load_stream_into_memory(StreamId(1))
        .expect("load comparison stream");
    assert!(resident
        .get_stream(StreamId(1))
        .unwrap()
        .tx_blocks
        .iter()
        .all(|block| block.status == BlockStatus::Loaded));
    let resident_counts = resident
        .get_stream(StreamId(1))
        .unwrap()
        .generators
        .iter()
        .map(|generator| {
            (
                *generator,
                resident
                    .get_generator(*generator)
                    .unwrap()
                    .transactions
                    .len(),
            )
        })
        .filter(|(_, count)| *count > 0)
        .collect::<HashMap<_, _>>();
    assert_eq!(streamed_counts, resident_counts);

    resident.drop_stream_from_memory(StreamId(1));
    let dropped = resident.get_stream(StreamId(1)).unwrap();
    assert!(!dropped.transactions_loaded);
    assert!(dropped
        .tx_blocks
        .iter()
        .all(|block| block.status == BlockStatus::Indexed));
    assert!(dropped.generators.iter().all(|generator| resident
        .get_generator(*generator)
        .unwrap()
        .transactions
        .is_empty()));
}

#[test]
fn streaming_visitor_records_callback_failures_without_loading_the_stream() {
    use crate::types::{BlockStatus, StreamId};

    let mut ftr = parse_ftr(PathBuf::from("./examples/test.ftr")).unwrap();
    let error = ftr
        .visit_stream_blocks(StreamId(1), |_, _| Err("cancelled by consumer".to_string()))
        .unwrap_err();
    assert_eq!(error, "cancelled by consumer");
    let stream = ftr.get_stream(StreamId(1)).unwrap();
    assert!(!stream.transactions_loaded);
    assert!(matches!(
        stream.tx_blocks.first().map(|block| &block.status),
        Some(BlockStatus::Error(message)) if message == "cancelled by consumer"
    ));
    assert!(stream
        .tx_blocks
        .iter()
        .skip(1)
        .all(|block| block.status == BlockStatus::Indexed));
}

#[test]
fn file_backed_relations_can_be_released_and_restored_for_legacy_loading() {
    use crate::types::StreamId;

    let mut ftr = parse_ftr(PathBuf::from("../examples/kanata-sample-2.ftr")).unwrap();
    assert!(ftr.tx_relations.is_empty());
    assert!(!ftr.relation_blocks.is_empty());
    ftr.load_relations_into_memory().unwrap();
    let relation_count = ftr.tx_relations.len();
    assert!(relation_count > 0);
    assert!(ftr.release_relations_if_unloaded());
    assert!(ftr.tx_relations.is_empty());

    ftr.load_stream_into_memory(StreamId(1)).unwrap();
    assert_eq!(ftr.tx_relations.len(), relation_count);
    assert!(ftr.get_stream(StreamId(1)).unwrap().transactions_loaded);
    assert!(!ftr.release_relations_if_unloaded());
}

#[test]
fn streamed_relation_directory_matches_resident_order_without_retention() {
    let mut ftr = parse_ftr(PathBuf::from("../examples/kanata-sample-2.ftr")).unwrap();
    assert!(ftr.tx_relations.is_empty());
    let mut streamed = Vec::new();
    let mut expected_ordinal = 0u64;
    ftr.visit_relation_blocks(|block, relations| {
        assert_eq!(block.ordinal, expected_ordinal);
        assert!(block.encoded_len > 0);
        expected_ordinal += 1;
        streamed.extend_from_slice(relations);
        Ok(())
    })
    .unwrap();
    assert!(ftr.tx_relations.is_empty());
    assert!(ftr
        .relation_blocks
        .iter()
        .all(|block| block.record_count.is_some()));
    assert_eq!(
        ftr.relation_blocks
            .iter()
            .map(|block| block.record_count.unwrap())
            .sum::<u64>(),
        streamed.len() as u64
    );

    ftr.load_relations_into_memory().unwrap();
    assert_eq!(ftr.tx_relations.as_slice(), streamed);
}
