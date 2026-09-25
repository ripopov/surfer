//! Deterministic packet/flit traffic; see chi_noc.md for model boundaries.
use vtr::{Reader, ScopeType, TxQuery, TxStatus, Value, Writer};

const CONTROLLERS: [&str; 6] = ["RNF0", "HNF0", "RNI0", "RNF1", "HNF1", "RNI1"];

fn route(src: usize, dst: usize) -> Vec<usize> {
    let (mut x, mut y) = (src % 3, src / 3);
    let (dx, dy) = (dst % 3, dst / 3);
    let mut hops = vec![src];
    while x != dx {
        x = if x < dx { x + 1 } else { x - 1 };
        hops.push(y * 3 + x);
    }
    while y != dy {
        y = if y < dy { y + 1 } else { y - 1 };
        hops.push(y * 3 + x);
    }
    hops
}

fn generate(path: &std::path::Path) {
    let mut w = Writer::create(path).unwrap();
    w.set_timescale(-9).unwrap();
    let noc = w.add_scope(None, "chi_noc", ScopeType::Module, "mesh_3x2").unwrap();
    let mut generators = Vec::new();
    for (src, name) in CONTROLLERS.iter().enumerate() {
        let scope = w.add_scope(Some(noc), name, ScopeType::Module, &name[..3]).unwrap();
        let stream = w.add_stream(Some(scope), "tx", "CHI_PACKET").unwrap();
        w.node_attr(stream, "max_outstanding", Value::U64(64))
            .unwrap();
        w.node_attr(scope, "router", Value::U64(src as u64))
            .unwrap();
        let ops: &[(&str, &str)] = match src % 3 {
            0 => &[("REQ", "ReadShared"), ("RSP", "SnpResp")],
            1 => &[("SNP", "SnpShared"), ("DAT", "CompData")],
            _ => &[("REQ", "ReadNoSnp")],
        };
        generators.push(
            ops.iter()
                .map(|&(ch, op)| (w.add_generator(stream, op).unwrap(), ch, op))
                .collect::<Vec<_>>(),
        );
    }
    for router in 0..6 {
        let r = w.add_scope(Some(noc), &format!("router_{router}"), ScopeType::Module, "router").unwrap();
        w.node_attr(r, "x", Value::U64(router % 3)).unwrap();
        w.node_attr(r, "y", Value::U64(router / 3)).unwrap();
    }
    w.node_attr(noc, "routing", Value::Text("XY".into()))
        .unwrap();
    let keys = [
        "channel", "opcode", "SrcID", "TgtID", "TxnID", "flits", "address",
    ]
    .map(|key| w.intern(key));
    let router_key = w.intern("router");
    let hop_key = w.intern("hop");
    let label_key = w.intern("vtr.label");
    let event_names = (0..6)
        .map(|r| w.intern(&format!("router_{r}")))
        .collect::<Vec<_>>();
    // Sparse traffic is readable in the snapshot; a separate burst reaches
    // exactly 64 overlapping packets in every source controller.
    for (base, count) in [(10, 4), (400, 64), (1000, 8)] {
        for slot in 0..count {
            for src in 0..6 {
                let choices = &generators[src];
                let (generator, channel, opcode) = choices[slot % choices.len()];
                let dst = match src % 3 {
                    1 => {
                        if slot % 2 == 0 {
                            3
                        } else {
                            0
                        }
                    }
                    _ => {
                        if src < 3 {
                            4
                        } else {
                            1
                        }
                    }
                };
                let start = base + slot as u64 * if count == 64 { 1 } else { 24 } + src as u64;
                let tx = w.begin_tx(generator, start).unwrap();
                let values = [
                    Value::Text(channel.into()),
                    Value::Text(opcode.into()),
                    Value::U64(src as u64),
                    Value::U64(dst as u64),
                    Value::U64(slot as u64),
                    Value::U64(1),
                    Value::U64(0x8000_0000 + slot as u64 * 64),
                ];
                for (key, value) in keys.iter().zip(values) {
                    w.tx_attr(tx, *key, &value).unwrap();
                }
                let packet_label = w.intern(&format!("packet {slot} {src} to {dst}"));
                w.tx_attr(tx, label_key, &Value::Str(packet_label)).unwrap();
                let hops = route(src, dst);
                for (hop, router) in hops.iter().enumerate() {
                    let hop_label = w.intern(&format!("packet {slot} hop {hop} at router {router}"));
                    w.tx_event(
                        tx,
                        start + hop as u64 * 40,
                        event_names[*router],
                        &[
                            (router_key, Value::U64(*router as u64)),
                            (hop_key, Value::U64(hop as u64)),
                            (label_key, Value::Str(hop_label)),
                        ],
                    )
                    .unwrap();
                }
                w.end_tx(tx, start + hops.len() as u64 * 40, TxStatus::Ok)
                    .unwrap();
            }
        }
    }
    w.close().unwrap();
}

fn verify(path: &std::path::Path) {
    let reader = Reader::open(path).unwrap();
    assert_eq!(reader.streams().count(), 6);
    for stream in reader.streams() {
        let packets = reader
            .transactions(&TxQuery {
                stream: Some(stream),
                ..Default::default()
            })
            .unwrap();
        for tx in &packets {
            assert!(tx.attrs.iter().any(|attr| reader.str(attr.key) == "vtr.label" && matches!(attr.value, Value::Str(_) | Value::Text(_))));
            assert!(tx.events.iter().all(|event| event.attrs.iter().any(|(key, value)| reader.str(*key) == "vtr.label" && matches!(value, Value::Str(_) | Value::Text(_)))));
        }
        assert_eq!(packets.len(), 76);
        let mut edges = Vec::new();
        for tx in packets {
            let attr = |name| {
                tx.attrs
                    .iter()
                    .find(|a| reader.str(a.key) == name)
                    .unwrap()
                    .value
                    .clone()
            };
            let Value::U64(src) = attr("SrcID") else {
                panic!()
            };
            let Value::U64(dst) = attr("TgtID") else {
                panic!()
            };
            assert_eq!(attr("flits"), Value::U64(1));
            let hops = route(src as usize, dst as usize);
            assert_eq!(tx.events.len(), hops.len());
            for (index, (event, router)) in tx.events.iter().zip(hops).enumerate() {
                assert_eq!(reader.str(event.name), format!("router_{router}"));
                assert_eq!(event.time, tx.begin + index as u64 * 40);
                assert!(event.time < tx.end);
            }
            edges.extend([(tx.begin, 1i32), (tx.end, -1)]);
        }
        edges.sort_unstable(); // End before begin at equal times: half-open lifetimes.
        let (mut active, mut peak) = (0, 0);
        for (_, delta) in edges {
            active += delta;
            peak = peak.max(active);
        }
        assert_eq!(active, 0);
        assert_eq!(peak, 64);
    }
}

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/chi_noc.vtr");
    generate(&path);
    verify(&path);
    println!(
        "{}: verified 456 packets, six controllers, peak 64 each",
        path.display()
    );
}

#[test]
fn committed_chi_noc_has_single_flits_routes_and_bounded_concurrency() {
    verify(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/chi_noc.vtr"));
}

#[test]
fn chi_noc_regenerates_identically() {
    let temp = tempfile::tempdir().unwrap();
    let generated = temp.path().join("chi_noc.vtr");
    generate(&generated);
    verify(&generated);
    let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/chi_noc.vtr");
    assert_eq!(std::fs::read(generated).unwrap(), std::fs::read(committed).unwrap());
}
