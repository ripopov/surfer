use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use lz4_flex::decompress_into;

use crate::cbor_decoder::CborDecoder;
use crate::types::{
    Attribute, AttributeType, BlockMeta, BlockStatus, DataType, Event, FtrResult, GeneratorId,
    NameId, RelationBlockMeta, StreamId, Timescale, Transaction, TransactionId, TxGenerator,
    TxRelation, TxStream, FTR,
};

const INFO_CHUNK: u64 = 6;
const DICTIONARY_CHUNK_UNCOMP: u64 = 8;
const DICTIONARY_CHUNK_COMP: u64 = 9;
const DIRECTORY_CHUNK_UNCOMP: u64 = 10;
const DIRECTORY_CHUNK_COMP: u64 = 11;
const TX_BLOCK_CHUNK_UNCOMP: u64 = 12;
const TX_BLOCK_CHUNK_COMP: u64 = 13;
const RELATIONSHIP_CHUNK_UNCOMP: u64 = 14;
const RELATIONSHIP_CHUNK_COMP: u64 = 15;

const STREAM: u64 = 16;
const GENERATOR: u64 = 17;

const EVENT_TAG: u64 = 6;
const BEGIN_TAG: u64 = 7;
const RECORD_TAG: u64 = 8;
const END_TAG: u64 = 9;

const BOOLEAN: u8 = 0;
const ENUMERATION: u8 = 1;
const INTEGER: u8 = 2;
const UNSIGNED: u8 = 3;
const FLOATING_POINT_NUMBER: u8 = 4;
const BIT_VECTOR: u8 = 5;
const LOGIC_VECTOR: u8 = 6;
const FIXED_POINT_INTEGER: u8 = 7;
const UNSIGNED_FIXED_POINT_INTEGER: u8 = 8;
const POINTER: u8 = 9;
const STRING: u8 = 10;
const TIME: u8 = 11;

pub struct FtrParser<'a> {
    ftr: &'a mut FTR,
    current_ends: HashMap<GeneratorId, Vec<u64>>,
}

impl<'a> FtrParser<'a> {
    pub fn new(ftr: &'a mut FTR) -> FtrParser<'a> {
        Self {
            ftr,
            current_ends: HashMap::new(),
        }
    }

    pub(super) fn load<R: Read + Seek>(&mut self, file: R) -> FtrResult<()> {
        let cbor_decoder = CborDecoder::new(file);
        self.parse_input(cbor_decoder)?;
        self.ftr.rebuild_relation_indices();
        Ok(())
    }

    //TODO change to work with buffered readers
    fn parse_input<R: Read + Seek>(
        &mut self,
        mut cbor_decoder: CborDecoder<R>,
    ) -> Result<(), String> {
        let tag = cbor_decoder.read_tag()?;
        if tag != 55799 {
            return Err("Not a valid FTR file".into());
        }
        let array_length = cbor_decoder.read_array_length()?;
        if array_length != -1 {
            return Err("Array does not have indefinite length. Not a valid FTR file!".into());
        }
        let mut next = cbor_decoder.peek();
        while next.is_ok() && next? != 0xff {
            let tag = cbor_decoder.read_tag()?;

            match tag as u64 {
                INFO_CHUNK => {
                    let mut cbd: CborDecoder<Cursor<Vec<u8>>> =
                        CborDecoder::new(Cursor::new(cbor_decoder.read_byte_string()?));
                    let len = cbd.read_array_length()?;
                    if len != 2 {
                        return Err(format!("Info chunk has wrong size. Expected 2 but found {len}. Not a valid FTR file."));
                    }

                    let time_scale = cbd.read_int()?;
                    self.ftr.time_scale = Timescale::get_timescale(time_scale);

                    let epoch_tag = cbd.read_tag()?;
                    if epoch_tag != 1 {
                        return Err(format!("Wrong epoch tag. Expected 1 but found {epoch_tag}. Not a valid FTR file!"));
                    }
                    let _creation_time = cbd.read_int()?;
                }
                DICTIONARY_CHUNK_UNCOMP => {
                    let mut cbd: CborDecoder<Cursor<Vec<u8>>> =
                        CborDecoder::new(Cursor::new(cbor_decoder.read_byte_string()?));
                    self.parse_dict(&mut cbd)?;
                }

                DICTIONARY_CHUNK_COMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 2 {
                        return Err(format!("Dictionary chunk has wrong size. Expected 2 but found {len}. Not a valid FTR file."));
                    }
                    let size = cbor_decoder.read_int()?; // uncompressed size
                    let bytes = cbor_decoder.read_byte_string()?;

                    let mut buf = vec![0u8; size as usize];
                    decompress_into(bytes.as_slice(), &mut buf).map_err(|e| e.to_string())?;

                    self.parse_dict(&mut CborDecoder::new(Cursor::new(buf)))?;
                }

                DIRECTORY_CHUNK_UNCOMP => {
                    let mut cbd = CborDecoder::new(Cursor::new(cbor_decoder.read_byte_string()?));
                    self.parse_dir(&mut cbd)?;
                }
                DIRECTORY_CHUNK_COMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 2 {
                        return Err(format!("Dictionary chunk has wrong size. Expected 2 but found {len}. Not a valid FTR file."));
                    }

                    let uncomp_size: usize = cbor_decoder.read_int()? as usize;
                    let mut buf = vec![0u8; uncomp_size];
                    let bytes = cbor_decoder.read_byte_string()?;
                    decompress_into(bytes.as_slice(), &mut buf).map_err(|e| e.to_string())?;

                    self.parse_dir(&mut CborDecoder::new(Cursor::new(buf)))?;
                }

                TX_BLOCK_CHUNK_UNCOMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 4 {
                        return Err(format!("Transaction block chunk has wrong size. Expected 4 but found {len}. Not a valid FTR file."));
                    }

                    let stream_id = StreamId(cbor_decoder.read_int()? as u64);
                    let start_time = cbor_decoder.read_int()? as u64;
                    let end_time = cbor_decoder.read_int()? as u64; // end time of block
                    if end_time > self.ftr.max_timestamp {
                        self.ftr.max_timestamp = end_time;
                    }

                    let encoded_offset = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|e| e.to_string())?;
                    let loaded_inline = self.ftr.path.is_none();

                    let uncompressed_len = if loaded_inline {
                        let bytes = cbor_decoder.read_byte_string()?;
                        let len = bytes.len() as u64;
                        let mut cbd = CborDecoder::new(Cursor::new(bytes));
                        self.parse_tx_block(&mut cbd)?;
                        self.ftr
                            .tx_streams
                            .get_mut(&stream_id)
                            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                            .transactions_loaded = true;
                        len
                    } else {
                        cbor_decoder.skip_byte_string_len()?
                    };
                    let encoded_end = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|e| e.to_string())?;
                    self.record_block(BlockMeta {
                        stream_id,
                        ordinal: 0,
                        encoded_offset,
                        encoded_len: encoded_end.saturating_sub(encoded_offset),
                        compressed: false,
                        uncompressed_len: Some(uncompressed_len),
                        start_time,
                        end_time,
                        status: if loaded_inline {
                            BlockStatus::Loaded
                        } else {
                            BlockStatus::Indexed
                        },
                    })?;
                }

                TX_BLOCK_CHUNK_COMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 5 {
                        return Err(format!("Transaction block chunk has wrong size. Expected 5 but found {len}. Not a valid FTR file."));
                    }

                    let stream_id = StreamId(cbor_decoder.read_int()? as u64);
                    let start_time = cbor_decoder.read_int()? as u64;
                    let end_time = cbor_decoder.read_int()? as u64; // end time of block

                    if end_time > self.ftr.max_timestamp {
                        self.ftr.max_timestamp = end_time;
                    }

                    let encoded_offset = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|e| e.to_string())?;
                    let uncomp_size = cbor_decoder.read_int()? as u64;
                    let loaded_inline = self.ftr.path.is_none();

                    if loaded_inline {
                        let compressed = cbor_decoder.read_byte_string()?;
                        let mut buf = vec![0u8; uncomp_size as usize];
                        decompress_into(&compressed, &mut buf).map_err(|e| e.to_string())?;
                        self.parse_tx_block(&mut CborDecoder::new(Cursor::new(buf)))?;
                        self.ftr
                            .tx_streams
                            .get_mut(&stream_id)
                            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                            .transactions_loaded = true;
                    } else {
                        cbor_decoder.skip_byte_string()?;
                    }
                    let encoded_end = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|e| e.to_string())?;
                    self.record_block(BlockMeta {
                        stream_id,
                        ordinal: 0,
                        encoded_offset,
                        encoded_len: encoded_end.saturating_sub(encoded_offset),
                        compressed: true,
                        uncompressed_len: Some(uncomp_size),
                        start_time,
                        end_time,
                        status: if loaded_inline {
                            BlockStatus::Loaded
                        } else {
                            BlockStatus::Indexed
                        },
                    })?;
                }

                RELATIONSHIP_CHUNK_UNCOMP => {
                    let encoded_offset = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|error| error.to_string())?;
                    let loaded_inline = self.ftr.path.is_none();
                    let uncompressed_len = if loaded_inline {
                        let bytes = cbor_decoder.read_byte_string()?;
                        let len = bytes.len() as u64;
                        self.parse_rel(&mut CborDecoder::new(Cursor::new(bytes)))?;
                        len
                    } else {
                        cbor_decoder.skip_byte_string_len()?
                    };
                    let encoded_end = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|error| error.to_string())?;
                    self.record_relation_block(RelationBlockMeta {
                        ordinal: 0,
                        encoded_offset,
                        encoded_len: encoded_end.saturating_sub(encoded_offset),
                        compressed: false,
                        uncompressed_len: Some(uncompressed_len),
                        record_count: None,
                        status: if loaded_inline {
                            BlockStatus::Loaded
                        } else {
                            BlockStatus::Indexed
                        },
                    });
                }

                RELATIONSHIP_CHUNK_COMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 2 {
                        return Err(format!("Relationship Chunk has wrong size. Expected 2 but found {len}. Not a valid FTR file."));
                    }
                    let encoded_offset = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|error| error.to_string())?;
                    let uncomp_size = cbor_decoder.read_int()? as u64;
                    let loaded_inline = self.ftr.path.is_none();
                    if loaded_inline {
                        let mut buf = vec![0u8; uncomp_size as usize];
                        let bytes = cbor_decoder.read_byte_string()?;
                        decompress_into(bytes.as_slice(), &mut buf).map_err(|e| e.to_string())?;
                        self.parse_rel(&mut CborDecoder::new(Cursor::new(buf)))?;
                    } else {
                        cbor_decoder.skip_byte_string()?;
                    }
                    let encoded_end = cbor_decoder
                        .input_stream
                        .stream_position()
                        .map_err(|error| error.to_string())?;
                    self.record_relation_block(RelationBlockMeta {
                        ordinal: 0,
                        encoded_offset,
                        encoded_len: encoded_end.saturating_sub(encoded_offset),
                        compressed: true,
                        uncompressed_len: Some(uncomp_size),
                        record_count: None,
                        status: if loaded_inline {
                            BlockStatus::Loaded
                        } else {
                            BlockStatus::Indexed
                        },
                    });
                }

                _ => return Err("Not a valid Tag!".into()),
            }

            next = cbor_decoder.peek();
        }
        Ok(())
    }

    fn record_block(&mut self, mut block: BlockMeta) -> FtrResult<()> {
        let stream = self
            .ftr
            .tx_streams
            .get_mut(&block.stream_id)
            .ok_or_else(|| format!("Cannot find stream with id {:?}", block.stream_id))?;
        block.ordinal = stream.tx_blocks.len() as u64;
        stream
            .tx_block_ids
            .push((block.encoded_offset, block.compressed));
        stream.tx_blocks.push(block);
        Ok(())
    }

    fn record_relation_block(&mut self, mut block: RelationBlockMeta) {
        block.ordinal = self.ftr.relation_blocks.len() as u64;
        self.ftr.relation_blocks.push(block);
    }

    fn parse_dict<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        let size = cbd.read_map_length()?;

        for _i in 0..size {
            let idx = cbd.read_int()? as u64;
            self.ftr
                .str_dict
                .insert(NameId(idx), Arc::from(cbd.read_text_string()?));
        }

        Ok(())
    }

    fn parse_dir<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        let size = cbd.read_array_length()?;
        if size < 0 {
            let mut next_dir = cbd.peek();
            while next_dir.is_ok() && next_dir? != 0xff {
                self.parse_dir_entry(cbd)?;

                next_dir = cbd.peek();
            }
        } else {
            for _i in 1..size {
                self.parse_dir_entry(cbd)?;
            }
        }
        Ok(())
    }

    fn parse_dir_entry<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        let dir_tag = cbd.read_tag()?;
        if dir_tag == STREAM as i64 {
            let len = cbd.read_array_length()?;
            if len != 3 {
                return Err("Directory Entry(Stream) has wrong size!".into());
            }
            let stream_id = StreamId(cbd.read_int()? as u64);

            let name_id = NameId(cbd.read_int()? as u64);
            let name = match self.ftr.str_dict.get(&name_id) {
                Some(n) => n,
                None => {
                    return Err(format!(
                        "There is no entry in the dictionary for id {:?}",
                        name_id
                    ))
                }
            };

            let kind_id = NameId(cbd.read_int()? as u64);
            let Some(kind) = self.ftr.str_dict.get(&kind_id) else {
                return Err(format!(
                    "There is no entry in the dictionary for id {:?}",
                    kind_id
                ));
            };

            self.ftr.tx_streams.insert(
                stream_id,
                TxStream {
                    id: stream_id,
                    name: name.to_string(),
                    kind: kind.to_string(),
                    generators: vec![],
                    transactions_loaded: false,
                    tx_blocks: vec![],
                    tx_block_ids: vec![],
                },
            );
        } else if dir_tag == GENERATOR as i64 {
            let len = cbd.read_array_length()?;
            if len != 3 {
                return Err("Directory entry(Generator) has wrong size!".into());
            }

            let gen_id = GeneratorId(cbd.read_int()? as u64);
            let name_id = NameId(cbd.read_int()? as u64);
            let Some(name) = self.ftr.str_dict.get(&name_id) else {
                return Err(format!(
                    "There is no entry in the dictionary for id {:?}",
                    name_id
                ));
            };

            let stream_id = StreamId(cbd.read_int()? as u64);

            let generator = TxGenerator {
                id: gen_id,
                name: name.to_string(),
                stream_id,
                transactions: Arc::new(vec![]),
            };

            self.ftr.tx_generators.insert(gen_id, generator);
            self.ftr
                .tx_streams
                .get_mut(&stream_id)
                .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                .generators
                .push(gen_id);
        }
        Ok(())
    }

    fn parse_tx_block<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        for transaction in self.decode_tx_block(cbd)? {
            if let Some(generator) = self.ftr.tx_generators.get_mut(&transaction.event.gen_id) {
                Arc::make_mut(&mut generator.transactions).push(transaction);
            }
        }
        Ok(())
    }

    fn decode_tx_block<R: Read + Seek>(
        &mut self,
        cbd: &mut CborDecoder<R>,
    ) -> FtrResult<Vec<Transaction>> {
        let size = cbd.read_array_length()?;
        if size != -1 {
            return Err("Transaction block does not have indefinite length!".into());
        }
        let mut transactions = Vec::new();

        while let Ok(next_tx) = cbd.peek() {
            if next_tx == 0xff {
                break;
            }
            let arr_len = cbd.read_array_length()?;

            let mut event = Event::default();
            let mut attributes: std::vec::Vec<Attribute> = vec![];

            for _ in 0..arr_len {
                let tag = cbd.read_tag()?;

                match tag as u64 {
                    EVENT_TAG => {
                        let len = cbd.read_array_length()?;
                        if len != 4 {
                            return Err(format!("Wrong size of event. Expected 4 but found {len}"));
                        }
                        let tx_id = TransactionId(cbd.read_int()? as u64);
                        let gen_id = GeneratorId(cbd.read_int()? as u64);
                        let start_time = cbd.read_int()? as u64;
                        let end_time = cbd.read_int()? as u64;
                        let new_event = Event {
                            tx_id,
                            gen_id,
                            start_time,
                            end_time,
                        };
                        event = new_event;
                    }
                    BEGIN_TAG => {
                        let len = cbd.read_array_length()?;
                        if len != 3 {
                            return Err(format!(
                                "Wrong size of begin attribute. Expected 3 but found {len}"
                            ));
                        }
                        let new_begin = self.parse_attribute(cbd, BEGIN_TAG)?;
                        attributes.push(new_begin);
                    }
                    RECORD_TAG => {
                        let len = cbd.read_array_length()?;
                        if len != 3 {
                            return Err(format!(
                                "Wrong size of record attribute. Expected 3 but found {len}"
                            ));
                        }
                        let new_record = self.parse_attribute(cbd, RECORD_TAG)?;
                        attributes.push(new_record);
                    }
                    END_TAG => {
                        let len = cbd.read_array_length()?;
                        if len != 3 {
                            return Err(format!(
                                "Wrong size of end attribute. Expected 3 but found {len}"
                            ));
                        }
                        let new_end = self.parse_attribute(cbd, END_TAG)?;
                        attributes.push(new_end);
                    }
                    invalid => return Err(format!("Invalid transaction block tag: {invalid}")),
                }
            }

            let gen_id = event.gen_id;
            let ends = self
                .current_ends
                .entry(gen_id)
                .or_insert_with(|| Vec::with_capacity(8));

            let row = match ends.iter().position(|end| *end <= event.start_time) {
                Some(row) => {
                    *ends.get_mut(row).ok_or("Cannot find row")? = event.end_time;
                    row
                }
                None => {
                    ends.push(event.end_time);
                    ends.len() - 1
                }
            };

            // Attach relations by index lookup instead of scanning the whole
            // relation list per transaction (was O(transactions x relations)).
            let out_relations = self.ftr.relations_from(event.tx_id).to_vec();
            let inc_relations = self.ftr.relations_to(event.tx_id).to_vec();

            let tx = Transaction {
                event,
                attributes,
                inc_relations,
                out_relations,
                row,
            };

            transactions.push(tx);
        }
        Ok(transactions)
    }

    fn parse_rel<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        let relations = self.decode_relations(cbd)?;
        Arc::make_mut(&mut self.ftr.tx_relations).extend(relations);
        Ok(())
    }

    fn decode_relations<R: Read + Seek>(
        &self,
        cbd: &mut CborDecoder<R>,
    ) -> FtrResult<Vec<TxRelation>> {
        let size = cbd.read_array_length()?;
        if size != -1 {
            return Err("Relation block does not have indefinite size!".into());
        }

        let mut relations = Vec::new();
        while let Ok(next_rel) = cbd.peek() {
            if next_rel == 0xff {
                break;
            }
            let len = cbd.read_array_length()?;
            if len != 5 && len != 3 {
                return Err(format!(
                    "Relation has wrong size. Expected 3 or 5 but found {len}"
                ));
            }
            let type_id = NameId(cbd.read_int()? as u64);
            let from_tx_id = TransactionId(cbd.read_int()? as u64);
            let to_tx_id = TransactionId(cbd.read_int()? as u64);

            // 5-element relations carry both stream ids explicitly (the form
            // the convention mandates). The 3-element fallback has to look each
            // stream up by its own transaction id -- previously both lookups
            // used the source id, so the sink stream was wrong.
            let (from_stream_id, to_stream_id) = if len > 3 {
                let from = StreamId(cbd.read_int()? as u64);
                let to = StreamId(cbd.read_int()? as u64);
                (from, to)
            } else {
                (
                    find_stream_for_tx(self.ftr, from_tx_id),
                    find_stream_for_tx(self.ftr, to_tx_id),
                )
            };

            let Some(rel_name) = self.ftr.str_dict.get(&type_id) else {
                return Err("Cannot find associated relation name".into());
            };

            relations.push(TxRelation {
                name: rel_name.clone(),
                source_tx_id: from_tx_id,
                sink_tx_id: to_tx_id,
                source_stream_id: from_stream_id,
                sink_stream_id: to_stream_id,
            });
        }

        Ok(relations)
    }

    pub(super) fn load_relations_from_file(&mut self, path: &Path) -> FtrResult<()> {
        let reader = File::open(path).map_err(|error| error.to_string())?;
        let blocks = self.ftr.relation_blocks.clone();
        self.ftr.tx_relations = Arc::new(Vec::new());
        self.ftr.rel_by_source.clear();
        self.ftr.rel_by_sink.clear();

        for block in blocks {
            let before = self.ftr.tx_relations.len();
            let result = self.decode_file_relation_block(&reader, &block);
            let record_count = result
                .as_ref()
                .ok()
                .map(|()| (self.ftr.tx_relations.len() - before) as u64);
            if let Some(stored) = self.ftr.relation_blocks.get_mut(block.ordinal as usize) {
                stored.status = match &result {
                    Ok(()) => BlockStatus::Loaded,
                    Err(error) => BlockStatus::Error(error.clone()),
                };
                if let Some(record_count) = record_count {
                    stored.record_count = Some(record_count);
                }
            }
            result?;
        }
        self.ftr.rebuild_relation_indices();
        Ok(())
    }

    fn decode_file_relation_block(
        &mut self,
        reader: &File,
        block: &RelationBlockMeta,
    ) -> FtrResult<()> {
        let relations = self.read_file_relation_block(reader, block)?;
        Arc::make_mut(&mut self.ftr.tx_relations).extend(relations);
        Ok(())
    }

    fn read_file_relation_block(
        &self,
        reader: &File,
        block: &RelationBlockMeta,
    ) -> FtrResult<Vec<TxRelation>> {
        let mut decoder = CborDecoder::new(reader);
        decoder
            .input_stream
            .seek(SeekFrom::Start(block.encoded_offset))
            .map_err(|error| error.to_string())?;
        if block.compressed {
            let uncompressed_len = decoder.read_int()?;
            let uncompressed_len = usize::try_from(uncompressed_len)
                .map_err(|_| "Invalid relationship block size".to_string())?;
            let compressed = decoder.read_byte_string()?;
            let mut bytes = vec![0; uncompressed_len];
            decompress_into(&compressed, &mut bytes).map_err(|error| error.to_string())?;
            self.decode_relations(&mut CborDecoder::new(Cursor::new(bytes)))
        } else {
            let bytes = decoder.read_byte_string()?;
            self.decode_relations(&mut CborDecoder::new(Cursor::new(bytes)))
        }
    }

    pub(super) fn visit_relation_blocks<F>(&mut self, mut visit: F) -> FtrResult<()>
    where
        F: FnMut(&RelationBlockMeta, &[TxRelation]) -> FtrResult<()>,
    {
        let path = self
            .ftr
            .path
            .clone()
            .ok_or_else(|| "Relationship blocks require a file-backed FTR".to_string())?;
        let blocks = self.ftr.relation_blocks.clone();
        let reader = File::open(path).map_err(|error| error.to_string())?;
        for block in blocks {
            let result = self
                .read_file_relation_block(&reader, &block)
                .and_then(|relations| {
                    let count = relations.len() as u64;
                    visit(&block, &relations).map(|()| count)
                });
            if let Some(stored) = self.ftr.relation_blocks.get_mut(block.ordinal as usize) {
                stored.status = match &result {
                    Ok(_) => BlockStatus::Indexed,
                    Err(error) => BlockStatus::Error(error.clone()),
                };
                if let Ok(count) = &result {
                    stored.record_count = Some(*count);
                }
            }
            result?;
        }
        Ok(())
    }

    pub(super) fn read_relation_block(&mut self, ordinal: u64) -> FtrResult<Vec<TxRelation>> {
        let path = self
            .ftr
            .path
            .clone()
            .ok_or_else(|| "Relationship blocks require a file-backed FTR".to_string())?;
        let block = self
            .ftr
            .relation_blocks
            .get(ordinal as usize)
            .cloned()
            .ok_or_else(|| format!("Cannot find relationship block {ordinal}"))?;
        let reader = File::open(path).map_err(|error| error.to_string())?;
        let result = self.read_file_relation_block(&reader, &block);
        if let Some(stored) = self.ftr.relation_blocks.get_mut(ordinal as usize) {
            stored.status = match &result {
                Ok(_) => BlockStatus::Indexed,
                Err(error) => BlockStatus::Error(error.clone()),
            };
            if let Ok(relations) = &result {
                stored.record_count = Some(relations.len() as u64);
            }
        }
        result
    }

    //loads the transactions of all generators of stream 'stream_id'
    pub(super) fn load_transactions(&mut self, stream_id: StreamId) -> FtrResult<()> {
        let Some(path) = &self.ftr.path else {
            return Err("Cannot load transaction when then input is not a file! \nTransactions should already be loaded.".into());
        };
        let reader = File::open(path).map_err(|e| e.to_string())?;

        let tx_block_ids = self
            .ftr
            .tx_streams
            .get(&stream_id)
            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
            .tx_block_ids
            .clone();

        for (ordinal, tx_block_id) in tx_block_ids.into_iter().enumerate() {
            let mut cbor_decoder = CborDecoder::new(&reader);

            let result: FtrResult<()> = (|| {
                cbor_decoder
                    .input_stream
                    .seek(SeekFrom::Start(tx_block_id.0))
                    .map_err(|e| e.to_string())?;

                if tx_block_id.1 {
                    let uncomp_size = cbor_decoder.read_int()?;

                    let mut buf = vec![0u8; uncomp_size as usize];
                    let bytes = cbor_decoder.read_byte_string()?;
                    decompress_into(bytes.as_slice(), &mut buf)
                        .map_err(|e| format!("Cannot decompress data correctly: {e}"))?;
                    self.parse_tx_block(&mut CborDecoder::new(Cursor::new(buf)))?;
                } else {
                    self.parse_tx_block(&mut CborDecoder::new(Cursor::new(
                        cbor_decoder.read_byte_string()?,
                    )))?;
                }
                Ok(())
            })();
            if let Some(block) = self
                .ftr
                .tx_streams
                .get_mut(&stream_id)
                .and_then(|stream| stream.tx_blocks.get_mut(ordinal))
            {
                block.status = match &result {
                    Ok(()) => BlockStatus::Loaded,
                    Err(error) => BlockStatus::Error(error.clone()),
                };
            }
            result?;
        }
        self.ftr
            .tx_streams
            .get_mut(&stream_id)
            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
            .transactions_loaded = true;
        Ok(())
    }

    /// Decodes one transaction block at a time without retaining its
    /// transactions in the generic FTR object graph. The callback completes
    /// before the batch is dropped and may return an error to cancel the scan.
    pub(super) fn visit_transaction_blocks<F>(
        &mut self,
        stream_id: StreamId,
        mut visit: F,
    ) -> FtrResult<()>
    where
        F: FnMut(&BlockMeta, &[Transaction]) -> FtrResult<()>,
    {
        let path = self
            .ftr
            .path
            .clone()
            .ok_or_else(|| "Transaction blocks require a file-backed FTR".to_string())?;
        let blocks = self
            .ftr
            .tx_streams
            .get(&stream_id)
            .ok_or_else(|| format!("Cannot find stream with id {stream_id:?}"))?
            .tx_blocks
            .clone();
        let reader = File::open(path).map_err(|error| error.to_string())?;
        self.current_ends.clear();
        for block in blocks {
            let result = self
                .decode_file_block(&reader, &block)
                .and_then(|transactions| visit(&block, &transactions));
            let status = match &result {
                Ok(()) => BlockStatus::Loaded,
                Err(error) => BlockStatus::Error(error.clone()),
            };
            if let Some(stored) = self
                .ftr
                .tx_streams
                .get_mut(&stream_id)
                .and_then(|stream| stream.tx_blocks.get_mut(block.ordinal as usize))
            {
                stored.status = status;
            }
            result?;
        }
        Ok(())
    }

    pub(super) fn read_transaction_block(
        &mut self,
        stream_id: StreamId,
        ordinal: u64,
    ) -> FtrResult<Vec<Transaction>> {
        let path = self
            .ftr
            .path
            .clone()
            .ok_or_else(|| "Transaction blocks require a file-backed FTR".to_string())?;
        let block = self
            .ftr
            .tx_streams
            .get(&stream_id)
            .and_then(|stream| stream.tx_blocks.get(ordinal as usize))
            .cloned()
            .ok_or_else(|| format!("Cannot find block {ordinal} for stream {stream_id}"))?;
        let reader = File::open(path).map_err(|error| error.to_string())?;
        self.current_ends.clear();
        let result = self.decode_file_block(&reader, &block);
        if let Some(stored) = self
            .ftr
            .tx_streams
            .get_mut(&stream_id)
            .and_then(|stream| stream.tx_blocks.get_mut(ordinal as usize))
        {
            stored.status = match &result {
                Ok(_) => BlockStatus::Loaded,
                Err(error) => BlockStatus::Error(error.clone()),
            };
        }
        result
    }

    fn decode_file_block(
        &mut self,
        reader: &File,
        block: &BlockMeta,
    ) -> FtrResult<Vec<Transaction>> {
        let mut decoder = CborDecoder::new(reader);
        decoder
            .input_stream
            .seek(SeekFrom::Start(block.encoded_offset))
            .map_err(|error| error.to_string())?;
        if block.compressed {
            let uncompressed_len = decoder.read_int()?;
            let uncompressed_len = usize::try_from(uncompressed_len)
                .map_err(|_| "Invalid transaction block size".to_string())?;
            let compressed = decoder.read_byte_string()?;
            let mut bytes = vec![0; uncompressed_len];
            decompress_into(&compressed, &mut bytes).map_err(|error| error.to_string())?;
            self.decode_tx_block(&mut CborDecoder::new(Cursor::new(bytes)))
        } else {
            let bytes = decoder.read_byte_string()?;
            self.decode_tx_block(&mut CborDecoder::new(Cursor::new(bytes)))
        }
    }

    fn parse_attribute<R: Read + Seek>(
        &self,
        cbd: &mut CborDecoder<R>,
        attribute_type: u64,
    ) -> FtrResult<Attribute> {
        let name_id = NameId(cbd.read_int()? as u64);
        let data_type = cbd.read_int()?;
        let data_type_with_value = match data_type as u8 {
            BOOLEAN => DataType::Boolean(cbd.read_boolean()?),
            ENUMERATION => DataType::Enumeration(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as u64))
                    .ok_or("Cannot find enum entry in string dictionary")?
                    .clone(),
            ),
            INTEGER => DataType::Integer(cbd.read_int()?),
            UNSIGNED => DataType::Unsigned(cbd.read_int()? as u64),
            FLOATING_POINT_NUMBER => DataType::FloatingPointNumber(cbd.read_float()?),
            BIT_VECTOR => DataType::BitVector(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as u64))
                    .ok_or("Cannot find bit vector entry in string dictionary")?
                    .clone(),
            ),
            LOGIC_VECTOR => DataType::LogicVector(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as u64))
                    .ok_or("Cannot find logic vector entry in string dictionary")?
                    .clone(),
            ),
            FIXED_POINT_INTEGER => DataType::FixedPointInteger(cbd.read_float()?),
            UNSIGNED_FIXED_POINT_INTEGER => DataType::UnsignedFixedPointInteger(cbd.read_float()?),
            POINTER => DataType::Pointer(cbd.read_int()? as u64),
            STRING => DataType::String(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as u64))
                    .ok_or("Cannot find string entry in string dictionary")?
                    .clone(),
            ),
            TIME => DataType::Time(cbd.read_int()? as u64),
            _ => DataType::Error,
        };

        let kind = match attribute_type {
            BEGIN_TAG => AttributeType::BEGIN,
            RECORD_TAG => AttributeType::RECORD,
            END_TAG => AttributeType::END,
            _ => AttributeType::NONE,
        };

        Ok(Attribute {
            kind,
            name: self
                .ftr
                .str_dict
                .get(&name_id)
                .ok_or("Cannot find attribute name")?
                .clone(),
            data_type: data_type_with_value,
        })
    }
}

/// Looks up the stream a transaction belongs to by scanning generators.
/// Only used for the rare 3-element relation fallback.
fn find_stream_for_tx(ftr: &FTR, tx_id: TransactionId) -> StreamId {
    for (gen_id, gen) in &ftr.tx_generators {
        for tx in gen.transactions.iter() {
            if tx.event.tx_id == tx_id && tx.event.gen_id == *gen_id {
                return gen.stream_id;
            }
        }
    }
    StreamId(0)
}

/// Attaches relations to the transactions they reference using sorted source
/// and sink permutations. This is O(relations log relations + transactions
/// log relations), without one hash-map allocation per transaction id.
pub(super) fn connect_relations_and_transactions(ftr: &mut FTR) {
    ftr.rebuild_relation_indices();
    let FTR {
        tx_generators,
        tx_relations,
        rel_by_source,
        rel_by_sink,
        ..
    } = ftr;
    for gen in tx_generators.values_mut() {
        for tx in Arc::make_mut(&mut gen.transactions).iter_mut() {
            tx.out_relations.clear();
            tx.inc_relations.clear();
            tx.out_relations.extend_from_slice(relation_range(
                tx_relations,
                rel_by_source,
                tx.event.tx_id,
                true,
            ));
            tx.inc_relations.extend_from_slice(relation_range(
                tx_relations,
                rel_by_sink,
                tx.event.tx_id,
                false,
            ));
        }
    }
}

fn relation_range<'a>(
    relations: &[TxRelation],
    permutation: &'a [usize],
    id: TransactionId,
    by_source: bool,
) -> &'a [usize] {
    let key = |index: usize| {
        if by_source {
            relations[index].source_tx_id
        } else {
            relations[index].sink_tx_id
        }
    };
    let start = permutation.partition_point(|index| key(*index) < id);
    let end = permutation.partition_point(|index| key(*index) <= id);
    &permutation[start..end]
}
