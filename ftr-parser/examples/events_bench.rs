//! Reproduces the performance issues described in
//! `surfer/docs/development/FTR_EVENTS.md` for the FTR transaction-events
//! convention.
//!
//! It synthesizes an FTR trace modeled on
//! `LWTR4SC/example/lwtr_event_example_wait.cpp`: a dual-issue CPU pipeline
//! whose instructions and memory transactions each record pipeline-stage
//! "events". Under the events convention every event is its own transaction
//! linked to its parent by a `parent_of` relation, so the trace has roughly
//! as many relations as transactions -- exactly the shape that makes relation
//! attachment quadratic in the parser.
//!
//! Run with an optional instruction count (default 6000):
//!   cargo run --release --example events_bench -- 6000

use std::io::Write;
use std::time::Instant;

use ftr_parser::parse::{parse_ftr, parse_ftr_from_bytes};
use ftr_parser::types::StreamId;

// ----------------------------------------------------------------------------
// Minimal CBOR / FTR writer (just enough to feed the parser).
// ----------------------------------------------------------------------------

/// CBOR major types.
const MT_UINT: u8 = 0;
const MT_NINT: u8 = 1;
const MT_BYTES: u8 = 2;
const MT_TEXT: u8 = 3;
const MT_ARRAY: u8 = 4;
const MT_MAP: u8 = 5;
const MT_TAG: u8 = 6;

/// FTR chunk tags.
const INFO_CHUNK: u64 = 6;
const DICT_CHUNK: u64 = 8;
const DIR_CHUNK: u64 = 10;
const TX_BLOCK_CHUNK: u64 = 12;
const REL_CHUNK: u64 = 14;
const STREAM_TAG: u64 = 16;
const GENERATOR_TAG: u64 = 17;

/// Event element tags inside a transaction.
const EVENT_TAG: u64 = 6;
const BEGIN_TAG: u64 = 7;
const RECORD_TAG: u64 = 8;

/// Attribute data types.
const TYPE_UNSIGNED: u8 = 3;
const TYPE_STRING: u8 = 10;

#[derive(Default)]
struct Cbor {
    buf: Vec<u8>,
}

impl Cbor {
    fn head(&mut self, major: u8, val: u64) {
        let mt = major << 5;
        if val < 24 {
            self.buf.push(mt | val as u8);
        } else if val < 0x100 {
            self.buf.push(mt | 24);
            self.buf.push(val as u8);
        } else if val < 0x1_0000 {
            self.buf.push(mt | 25);
            self.buf.extend_from_slice(&(val as u16).to_be_bytes());
        } else if val < 0x1_0000_0000 {
            self.buf.push(mt | 26);
            self.buf.extend_from_slice(&(val as u32).to_be_bytes());
        } else {
            self.buf.push(mt | 27);
            self.buf.extend_from_slice(&val.to_be_bytes());
        }
    }

    fn uint(&mut self, val: u64) {
        self.head(MT_UINT, val);
    }

    fn int(&mut self, val: i64) {
        if val >= 0 {
            self.head(MT_UINT, val as u64);
        } else {
            self.head(MT_NINT, (-1 - val) as u64);
        }
    }

    fn tag(&mut self, val: u64) {
        self.head(MT_TAG, val);
    }

    fn array(&mut self, len: u64) {
        self.head(MT_ARRAY, len);
    }

    fn array_indef(&mut self) {
        self.buf.push((MT_ARRAY << 5) | 31);
    }

    fn map(&mut self, len: u64) {
        self.head(MT_MAP, len);
    }

    fn brk(&mut self) {
        self.buf.push(0xff);
    }

    fn text(&mut self, s: &str) {
        self.head(MT_TEXT, s.len() as u64);
        self.buf.extend_from_slice(s.as_bytes());
    }

    fn bytes(&mut self, b: &[u8]) {
        self.head(MT_BYTES, b.len() as u64);
        self.buf.extend_from_slice(b);
    }
}

/// String dictionary: assigns a stable id to every interned string.
#[derive(Default)]
struct Dict {
    ids: std::collections::HashMap<String, u64>,
    order: Vec<(u64, String)>,
    next: u64,
}

impl Dict {
    fn id(&mut self, s: &str) -> u64 {
        if let Some(id) = self.ids.get(s) {
            return *id;
        }
        let id = self.next;
        self.next += 1;
        self.ids.insert(s.to_string(), id);
        self.order.push((id, s.to_string()));
        id
    }
}

/// An attribute to write into a transaction.
enum Attr {
    BeginStr(u64, u64), // name_id, value_id
    RecordU(u64, u64),  // name_id, value
}

/// Append one transaction (event + attributes) to a tx-block buffer.
fn write_tx(c: &mut Cbor, tx_id: u64, gen_id: u64, start: u64, end: u64, attrs: &[Attr]) {
    c.array(1 + attrs.len() as u64);
    // event element
    c.tag(EVENT_TAG);
    c.array(4);
    c.uint(tx_id);
    c.uint(gen_id);
    c.uint(start);
    c.uint(end);
    // attributes
    for a in attrs {
        match *a {
            Attr::BeginStr(name, val) => {
                c.tag(BEGIN_TAG);
                c.array(3);
                c.uint(name);
                c.uint(TYPE_STRING as u64);
                c.uint(val);
            }
            Attr::RecordU(name, val) => {
                c.tag(RECORD_TAG);
                c.array(3);
                c.uint(name);
                c.uint(TYPE_UNSIGNED as u64);
                c.uint(val);
            }
        }
    }
}

/// Wrap a chunk body as `tag(chunk) byte_string(body)` and append to `out`.
fn emit_chunk(out: &mut Cbor, chunk_tag: u64, body: &[u8]) {
    out.tag(chunk_tag);
    out.bytes(body);
}

fn build_ftr(num_instr: u64) -> (Vec<u8>, u64, u64) {
    let mut dict = Dict::default();

    // Streams / generators (mirrors the lwtr example layout).
    const CPU_STREAM: u64 = 1;
    const MEM_STREAM: u64 = 2;
    const GEN_INSTR: u64 = 10;
    const GEN_INSTR_EVENTS: u64 = 11;
    const GEN_BUS: u64 = 12;
    const GEN_BUS_EVENTS: u64 = 13;

    let cpu_name = dict.id("CPU_Core");
    let mem_name = dict.id("Memory");
    let kind = dict.id("transactions");
    let g_instr = dict.id("instruction");
    let g_instr_ev = dict.id("instruction.events");
    let g_bus = dict.id("bus_transaction");
    let g_bus_ev = dict.id("bus_transaction.events");

    let a_mnemonic = dict.id("mnemonic");
    let a_pc = dict.id("pc");
    let a_name = dict.id("name");
    let a_stage_id = dict.id("stage_id");
    let a_op = dict.id("op");
    let a_addr = dict.id("addr");
    let a_fabric = dict.id("fabric_node");
    let rel_parent_of = dict.id("parent_of");

    let mnemonics: Vec<u64> = ["ADD", "SUB", "LW", "ADDI", "SW", "BEQ", "AND", "OR"]
        .iter()
        .map(|m| dict.id(m))
        .collect();
    let stages: Vec<u64> = ["IF", "ID", "EX", "MEM", "WB"]
        .iter()
        .map(|s| dict.id(s))
        .collect();
    let mem_stages: Vec<u64> = ["REQ", "ARB", "ROUTE", "ACCESS", "RESP"]
        .iter()
        .map(|s| dict.id(s))
        .collect();
    let op_load = dict.id("load");

    // Transaction bodies, per stream, plus the relation list.
    let mut cpu_txs = Cbor::default();
    cpu_txs.array_indef();
    let mut mem_txs = Cbor::default();
    mem_txs.array_indef();
    let mut rels = Cbor::default();
    rels.array_indef();

    let mut next_tx: u64 = 1;
    let mut num_rel: u64 = 0;
    let mut max_time: u64 = 0;

    let add_rel = |rels: &mut Cbor, src_tx: u64, src_stream: u64, snk_tx: u64, snk_stream: u64| {
        rels.array(5);
        rels.uint(rel_parent_of);
        rels.uint(src_tx);
        rels.uint(snk_tx);
        rels.uint(src_stream);
        rels.uint(snk_stream);
    };

    for i in 0..num_instr {
        let t = i * 50;
        let parent = next_tx;
        next_tx += 1;

        write_tx(
            &mut cpu_txs,
            parent,
            GEN_INSTR,
            t,
            t + 50,
            &[
                Attr::BeginStr(a_mnemonic, mnemonics[(i as usize) % mnemonics.len()]),
                Attr::RecordU(a_pc, 0x1000 + i * 4),
            ],
        );

        // 5 pipeline-stage events, each a zero-duration tx + a parent_of relation.
        for (s, &stage) in stages.iter().enumerate() {
            let ev = next_tx;
            next_tx += 1;
            let et = t + (s as u64) * 10;
            write_tx(
                &mut cpu_txs,
                ev,
                GEN_INSTR_EVENTS,
                et,
                et,
                &[
                    Attr::BeginStr(a_name, stage),
                    Attr::RecordU(a_stage_id, s as u64),
                ],
            );
            add_rel(&mut rels, parent, CPU_STREAM, ev, CPU_STREAM);
            num_rel += 1;
        }

        // Every third instruction is a memory op with its own event stream.
        if i % 3 == 0 {
            let mem = next_tx;
            next_tx += 1;
            let mt = t + 30;
            write_tx(
                &mut mem_txs,
                mem,
                GEN_BUS,
                mt,
                mt + 40,
                &[
                    Attr::BeginStr(a_op, op_load),
                    Attr::RecordU(a_addr, 0x2000 + i),
                ],
            );
            for (s, &stage) in mem_stages.iter().enumerate() {
                let ev = next_tx;
                next_tx += 1;
                let et = mt + (s as u64) * 8;
                write_tx(
                    &mut mem_txs,
                    ev,
                    GEN_BUS_EVENTS,
                    et,
                    et,
                    &[
                        Attr::BeginStr(a_name, stage),
                        Attr::RecordU(a_fabric, s as u64),
                    ],
                );
                add_rel(&mut rels, mem, MEM_STREAM, ev, MEM_STREAM);
                num_rel += 1;
            }
            // parent_of: instruction (parent) -> memory transaction (child).
            add_rel(&mut rels, parent, CPU_STREAM, mem, MEM_STREAM);
            num_rel += 1;
            max_time = max_time.max(mt + 40);
        }
        max_time = max_time.max(t + 50);
    }

    cpu_txs.brk();
    mem_txs.brk();
    rels.brk();

    // Dictionary body.
    let mut dict_body = Cbor::default();
    dict_body.map(dict.order.len() as u64);
    for (id, s) in &dict.order {
        dict_body.uint(*id);
        dict_body.text(s);
    }

    // Directory body (indefinite array of stream / generator entries).
    let mut dir_body = Cbor::default();
    dir_body.array_indef();
    // streams
    for (sid, nid) in [(CPU_STREAM, cpu_name), (MEM_STREAM, mem_name)] {
        dir_body.tag(STREAM_TAG);
        dir_body.array(3);
        dir_body.uint(sid);
        dir_body.uint(nid);
        dir_body.uint(kind);
    }
    // generators
    for (gid, nid, sid) in [
        (GEN_INSTR, g_instr, CPU_STREAM),
        (GEN_INSTR_EVENTS, g_instr_ev, CPU_STREAM),
        (GEN_BUS, g_bus, MEM_STREAM),
        (GEN_BUS_EVENTS, g_bus_ev, MEM_STREAM),
    ] {
        dir_body.tag(GENERATOR_TAG);
        dir_body.array(3);
        dir_body.uint(gid);
        dir_body.uint(nid);
        dir_body.uint(sid);
    }
    dir_body.brk();

    // Info body: [timescale_exp, tag(1) creation_time].
    let mut info_body = Cbor::default();
    info_body.array(2);
    info_body.int(-12);
    info_body.tag(1);
    info_body.uint(0);

    // Assemble the file.
    let mut out = Cbor::default();
    out.tag(55799);
    out.array_indef();

    emit_chunk(&mut out, INFO_CHUNK, &info_body.buf);
    emit_chunk(&mut out, DICT_CHUNK, &dict_body.buf);
    emit_chunk(&mut out, DIR_CHUNK, &dir_body.buf);

    // Tx-block chunk: tag(12) array(4)[stream, start, end, byte_string(body)].
    for (stream_id, body) in [(CPU_STREAM, &cpu_txs.buf), (MEM_STREAM, &mem_txs.buf)] {
        out.tag(TX_BLOCK_CHUNK);
        out.array(4);
        out.uint(stream_id);
        out.uint(0);
        out.uint(max_time);
        out.bytes(body);
    }

    emit_chunk(&mut out, REL_CHUNK, &rels.buf);

    out.brk();

    (out.buf, next_tx - 1, num_rel)
}

fn main() {
    let num_instr: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(6000);

    println!("Generating FTR trace with {num_instr} instructions...");
    let (bytes, num_tx, num_rel) = build_ftr(num_instr);
    println!(
        "  {} transactions, {} relations, {:.1} MiB",
        num_tx,
        num_rel,
        bytes.len() as f64 / (1024.0 * 1024.0)
    );

    let path = std::env::temp_dir().join("ftr_events_bench.ftr");
    {
        let mut f = std::fs::File::create(&path).expect("create file");
        f.write_all(&bytes).expect("write file");
    }

    // Path 1: file-based parse + lazy stream load (the Surfer load path).
    let t0 = Instant::now();
    let mut ftr = parse_ftr(path.clone()).expect("parse_ftr");
    let t_header = t0.elapsed();

    let t1 = Instant::now();
    ftr.load_stream_into_memory(StreamId(1))
        .expect("load stream 1");
    ftr.load_stream_into_memory(StreamId(2))
        .expect("load stream 2");
    let t_load = t1.elapsed();

    // Path 2: whole-file in-memory parse (connect_relations_and_transactions).
    let t2 = Instant::now();
    let ftr2 = parse_ftr_from_bytes(bytes).expect("parse_ftr_from_bytes");
    let t_bytes = t2.elapsed();

    let loaded_tx: usize = ftr
        .tx_generators
        .values()
        .map(|g| g.transactions.len())
        .sum();
    let loaded_tx2: usize = ftr2
        .tx_generators
        .values()
        .map(|g| g.transactions.len())
        .sum();

    // Correctness: the first instruction (tx id 1) is a memory op (i % 3 == 0),
    // so it has 5 pipeline-stage events plus 1 child memory transaction = 6
    // outgoing parent_of relations, all resolvable through the index API. Each
    // of its event transactions has exactly 1 incoming parent_of relation back
    // to it.
    let instr_gen = ftr
        .get_generator_from_name(Some(StreamId(1)), "instruction".to_string())
        .expect("instruction generator");
    let first = &instr_gen.transactions[0];
    assert_eq!(first.get_tx_id().0, 1);
    assert_eq!(
        first.out_relations.len(),
        6,
        "first instruction parent_of count"
    );
    for &idx in &first.out_relations {
        let rel = ftr.get_relation(idx).expect("relation resolves");
        assert_eq!(rel.name.as_ref(), "parent_of");
        assert_eq!(rel.source_tx_id, first.get_tx_id());
    }
    let events_gen = ftr
        .get_generator_from_name(Some(StreamId(1)), "instruction.events".to_string())
        .expect("events generator");
    let first_event = &events_gen.transactions[0];
    assert_eq!(first_event.inc_relations.len(), 1);
    let back = ftr
        .get_relation(first_event.inc_relations[0])
        .expect("incoming relation resolves");
    assert_eq!(back.source_tx_id, first.get_tx_id());
    assert_eq!(back.sink_tx_id, first_event.get_tx_id());
    println!("correctness checks passed (relations resolve through index API)");

    println!();
    println!("parse_ftr (header only):        {t_header:?}");
    println!("load_stream_into_memory (x2):   {t_load:?}   ({loaded_tx} txs)");
    println!("parse_ftr_from_bytes (all):     {t_bytes:?}   ({loaded_tx2} txs)");
}
