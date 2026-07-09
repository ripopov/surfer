use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use lz4_flex::decompress_into;

use crate::cbor_decoder::CborDecoder;
use crate::types::{
    Attribute, AttributeType, DataType, Event, FtrResult, GeneratorId, NameId, StreamId, Timescale,
    Transaction, TransactionId, TxGenerator, TxRelation, TxStream, FTR,
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
}

impl<'a> FtrParser<'a> {
    pub fn new(ftr: &'a mut FTR) -> FtrParser<'a> {
        Self { ftr }
    }

    pub(super) fn load<R: Read + Seek>(&mut self, file: R) -> FtrResult<()> {
        let cbor_decoder = CborDecoder::new(file);
        self.parse_input(cbor_decoder)
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

                    let stream_id = StreamId(cbor_decoder.read_int()? as usize);
                    let _start_time = cbor_decoder.read_int()?; // start time of block
                    let end_time = cbor_decoder.read_int()? as u64; // end time of block
                    if end_time > self.ftr.max_timestamp {
                        self.ftr.max_timestamp = end_time;
                    }

                    self.ftr
                        .tx_streams
                        .get_mut(&stream_id)
                        .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                        .tx_block_ids
                        .push((
                            cbor_decoder
                                .input_stream
                                .stream_position()
                                .map_err(|e| e.to_string())?,
                            false,
                        ));

                    if self.ftr.path.is_none() {
                        let mut cbd =
                            CborDecoder::new(Cursor::new(cbor_decoder.read_byte_string()?));
                        self.parse_tx_block(&mut cbd)?;
                        self.ftr
                            .tx_streams
                            .get_mut(&stream_id)
                            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                            .transactions_loaded = true;
                    } else {
                        cbor_decoder.skip_byte_string()?; // we don't want to load the transactions right now, so we just skip this whole block
                    }
                }

                TX_BLOCK_CHUNK_COMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 5 {
                        return Err(format!("Transaction block chunk has wrong size. Expected 5 but found {len}. Not a valid FTR file."));
                    }

                    let stream_id = StreamId(cbor_decoder.read_int()? as usize);
                    let _start_time = cbor_decoder.read_int()?; // start time of block
                    let end_time = cbor_decoder.read_int()? as u64; // end time of block

                    if end_time > self.ftr.max_timestamp {
                        self.ftr.max_timestamp = end_time;
                    }

                    self.ftr
                        .tx_streams
                        .get_mut(&stream_id)
                        .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                        .tx_block_ids
                        .push((
                            cbor_decoder
                                .input_stream
                                .stream_position()
                                .map_err(|e| e.to_string())?,
                            true,
                        ));
                    let _uncomp_size = cbor_decoder.read_int();

                    if self.ftr.path.is_none() {
                        let mut cbd =
                            CborDecoder::new(Cursor::new(cbor_decoder.read_byte_string()?));
                        self.parse_tx_block(&mut cbd)?;
                        self.ftr
                            .tx_streams
                            .get_mut(&stream_id)
                            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
                            .transactions_loaded = true;
                    } else {
                        cbor_decoder.skip_byte_string()?;
                    }
                }

                RELATIONSHIP_CHUNK_UNCOMP => {
                    let mut cbd = CborDecoder::new(Cursor::new(cbor_decoder.read_byte_string()?));
                    self.parse_rel(&mut cbd)?;
                }

                RELATIONSHIP_CHUNK_COMP => {
                    let len = cbor_decoder.read_array_length()?;
                    if len != 2 {
                        return Err(format!("Relationship Chunk has wrong size. Expected 2 but found {len}. Not a valid FTR file."));
                    }
                    let uncomp_size = cbor_decoder.read_int()?;
                    let mut buf = vec![0u8; uncomp_size as usize];
                    let bytes = cbor_decoder.read_byte_string()?;
                    decompress_into(bytes.as_slice(), &mut buf).map_err(|e| e.to_string())?;

                    self.parse_rel(&mut CborDecoder::new(Cursor::new(buf)))?;
                }

                _ => return Err("Not a valid Tag!".into()),
            }

            next = cbor_decoder.peek();
        }
        Ok(())
    }

    fn parse_dict<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        let size = cbd.read_map_length()?;

        for _i in 0..size {
            let idx = cbd.read_int()? as usize;
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
            let stream_id = StreamId(cbd.read_int()? as usize);

            let name_id = NameId(cbd.read_int()? as usize);
            let name = match self.ftr.str_dict.get(&name_id) {
                Some(n) => n,
                None => {
                    return Err(format!(
                        "There is no entry in the dictionary for id {:?}",
                        name_id
                    ))
                }
            };

            let kind_id = NameId(cbd.read_int()? as usize);
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
                    tx_block_ids: vec![],
                },
            );
        } else if dir_tag == GENERATOR as i64 {
            let len = cbd.read_array_length()?;
            if len != 3 {
                return Err("Directory entry(Generator) has wrong size!".into());
            }

            let gen_id = GeneratorId(cbd.read_int()? as usize);
            let name_id = NameId(cbd.read_int()? as usize);
            let Some(name) = self.ftr.str_dict.get(&name_id) else {
                return Err(format!(
                    "There is no entry in the dictionary for id {:?}",
                    name_id
                ));
            };

            let stream_id = StreamId(cbd.read_int()? as usize);

            let generator = TxGenerator {
                id: gen_id,
                name: name.to_string(),
                stream_id,
                transactions: vec![],
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
        let size = cbd.read_array_length()?;
        if size != -1 {
            return Err("Transaction block does not have indefinite length!".into());
        }

        let mut current_ends: HashMap<GeneratorId, Vec<u64>> = HashMap::new();

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
                        let tx_id = TransactionId(cbd.read_int()? as usize);
                        let gen_id = GeneratorId(cbd.read_int()? as usize);
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
            let ends = current_ends
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
            let out_relations = self
                .ftr
                .rel_by_source
                .get(&event.tx_id)
                .cloned()
                .unwrap_or_default();
            let inc_relations = self
                .ftr
                .rel_by_sink
                .get(&event.tx_id)
                .cloned()
                .unwrap_or_default();

            let tx = Transaction {
                event,
                attributes,
                inc_relations,
                out_relations,
                row,
            };

            if let Some(generator) = self.ftr.tx_generators.get_mut(&tx.event.gen_id) {
                generator.transactions.push(tx);
            }
        }
        Ok(())
    }

    fn parse_rel<R: Read + Seek>(&mut self, cbd: &mut CborDecoder<R>) -> FtrResult<()> {
        let size = cbd.read_array_length()?;
        if size != -1 {
            return Err("Relation block does not have indefinite size!".into());
        }

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
            let type_id = NameId(cbd.read_int()? as usize);
            let from_tx_id = TransactionId(cbd.read_int()? as usize);
            let to_tx_id = TransactionId(cbd.read_int()? as usize);

            // 5-element relations carry both stream ids explicitly (the form
            // the convention mandates). The 3-element fallback has to look each
            // stream up by its own transaction id -- previously both lookups
            // used the source id, so the sink stream was wrong.
            let (from_stream_id, to_stream_id) = if len > 3 {
                let from = StreamId(cbd.read_int()? as usize);
                let to = StreamId(cbd.read_int()? as usize);
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

            let tx_relation = TxRelation {
                name: rel_name.clone(),
                source_tx_id: from_tx_id,
                sink_tx_id: to_tx_id,
                source_stream_id: from_stream_id,
                sink_stream_id: to_stream_id,
            };

            let index = self.ftr.tx_relations.len();
            self.ftr
                .rel_by_source
                .entry(from_tx_id)
                .or_default()
                .push(index);
            self.ftr
                .rel_by_sink
                .entry(to_tx_id)
                .or_default()
                .push(index);
            self.ftr.tx_relations.push(tx_relation);
        }

        Ok(())
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

        for tx_block_id in tx_block_ids {
            let mut cbor_decoder = CborDecoder::new(&reader);

            cbor_decoder
                .input_stream
                .seek(SeekFrom::Start(tx_block_id.0))
                .map_err(|e| e.to_string())?;

            if tx_block_id.1 {
                let uncomp_size = cbor_decoder.read_int()?;

                let mut buf = vec![0u8; uncomp_size as usize];
                let bytes = cbor_decoder.read_byte_string()?;
                match decompress_into(bytes.as_slice(), &mut buf) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(format!("Cannot decompress data correctly: {e}"));
                    }
                }

                self.parse_tx_block(&mut CborDecoder::new(Cursor::new(buf)))?;
            } else {
                self.parse_tx_block(&mut CborDecoder::new(Cursor::new(
                    cbor_decoder.read_byte_string()?,
                )))?;
            }
        }
        self.ftr
            .tx_streams
            .get_mut(&stream_id)
            .ok_or_else(|| format!("Cannot find stream with id {:?}", stream_id))?
            .transactions_loaded = true;
        Ok(())
    }

    fn parse_attribute<R: Read + Seek>(
        &self,
        cbd: &mut CborDecoder<R>,
        attribute_type: u64,
    ) -> FtrResult<Attribute> {
        let name_id = NameId(cbd.read_int()? as usize);
        let data_type = cbd.read_int()?;
        let data_type_with_value = match data_type as u8 {
            BOOLEAN => DataType::Boolean(cbd.read_boolean()?),
            ENUMERATION => DataType::Enumeration(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as usize))
                    .ok_or("Cannot find enum entry in string dictionary")?
                    .clone(),
            ),
            INTEGER => DataType::Integer(cbd.read_int()?),
            UNSIGNED => DataType::Unsigned(cbd.read_int()? as u64),
            FLOATING_POINT_NUMBER => DataType::FloatingPointNumber(cbd.read_float()?),
            BIT_VECTOR => DataType::BitVector(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as usize))
                    .ok_or("Cannot find bit vector entry in string dictionary")?
                    .clone(),
            ),
            LOGIC_VECTOR => DataType::LogicVector(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as usize))
                    .ok_or("Cannot find logic vector entry in string dictionary")?
                    .clone(),
            ),
            FIXED_POINT_INTEGER => DataType::FixedPointInteger(cbd.read_float()?),
            UNSIGNED_FIXED_POINT_INTEGER => DataType::UnsignedFixedPointInteger(cbd.read_float()?),
            POINTER => DataType::Pointer(cbd.read_int()? as u64),
            STRING => DataType::String(
                self.ftr
                    .str_dict
                    .get(&NameId(cbd.read_int()? as usize))
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
        for tx in &gen.transactions {
            if tx.event.tx_id == tx_id && tx.event.gen_id == *gen_id {
                return gen.stream_id;
            }
        }
    }
    StreamId(0)
}

/// Attaches relations to the transactions they reference using the prebuilt
/// source/sink index maps. This is O(transactions + relations) rather than the
/// previous O(transactions x relations) scan.
pub(super) fn connect_relations_and_transactions(ftr: &mut FTR) {
    let FTR {
        tx_generators,
        rel_by_source,
        rel_by_sink,
        ..
    } = ftr;
    for gen in tx_generators.values_mut() {
        for tx in gen.transactions.iter_mut() {
            tx.out_relations.clear();
            tx.inc_relations.clear();
            if let Some(idxs) = rel_by_source.get(&tx.event.tx_id) {
                tx.out_relations.extend_from_slice(idxs);
            }
            if let Some(idxs) = rel_by_sink.get(&tx.event.tx_id) {
                tx.inc_relations.extend_from_slice(idxs);
            }
        }
    }
}
