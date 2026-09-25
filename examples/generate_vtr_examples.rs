//! Reproduce the supplemental transaction examples (not simulator waveforms).
use vtr::{
    Direction as VtrDirection, Reader, ScopeType as VtrScopeType, ScopeType, SignalKind, TxQuery,
    Value, VarType as VtrVarType, Writer,
};
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    combined(root.join("combined.vtr"));
    transactions(root.join("transactions.vtr"));
}
fn combined(path: std::path::PathBuf) {
    let mut writer = Writer::create(&path).unwrap();
    let top = writer.add_scope(None, "top", VtrScopeType::Module, "top").unwrap();
    let (_, count) = writer.add_var(
        Some(top),
        "count",
        VtrVarType::Logic,
        VtrDirection::Output,
        SignalKind::Bits {
            width: 8,
            states: 4,
        },
    ).unwrap();
    let stream = writer.add_stream(None, "cpu", "PIPELINE").unwrap();
    let generator = writer.add_generator(stream, "issue").unwrap();
    let key = writer.intern("opcode");
    let label = writer.intern("vtr.label");
    let event_name = writer.intern("retire");
    let stage_name = writer.intern("execute");
    let lane_name = writer.intern("alu");
    writer.set_time(0).unwrap();
    writer.emit_u64(count, 3).unwrap();
    writer.set_time(4).unwrap();
    writer.emit_u64(count, 7).unwrap();
    let tx = writer.begin_tx(generator, 1).unwrap();
    writer.set_tx_kind(tx, vtr::TxKind::Internal).unwrap();
    writer
        .tx_event(tx, 2, event_name, &[(label, Value::Text("retire add #0".into()))])
        .unwrap();
    writer
        .tx_stage(tx, stage_name, lane_name, 1, 2, &[(label, Value::Text("execute add #0".into()))])
        .unwrap();
    writer
        .tx_attr(tx, key, &vtr::Value::Text("add".into()))
        .unwrap();
    writer
        .tx_attr(tx, label, &Value::Text("add #0".into()))
        .unwrap();
    writer.end_tx(tx, 3, vtr::TxStatus::Ok).unwrap();
    writer.close().unwrap();
    verify_labels(&path);
}
fn transactions(path: std::path::PathBuf) {
    let mut writer = Writer::create(&path).unwrap();
    let stream = writer.add_stream(None, "cpu", "PIPELINE").unwrap();
    let generator = writer.add_generator(stream, "issue").unwrap();
    writer
        .add_scope(None, "top", ScopeType::Module, "top")
        .unwrap();
    let key = writer.intern("opcode");
    let label = writer.intern("vtr.label");
    let event_name = writer.intern("retire");
    let stage_name = writer.intern("execute");
    let lane_name = writer.intern("alu");
    let first = writer.begin_tx(generator, 2).unwrap();
    writer.set_tx_kind(first, vtr::TxKind::Internal).unwrap();
    writer
        .tx_event(first, 3, event_name, &[(label, Value::Text("retire add #0".into()))])
        .unwrap();
    writer
        .tx_stage(first, stage_name, lane_name, 2, 5, &[(label, Value::Text("execute add #0".into()))])
        .unwrap();
    writer
        .tx_attr(first, key, &Value::Text("add".into()))
        .unwrap();
    writer
        .tx_attr(first, label, &Value::Text("add #0".into()))
        .unwrap();
    writer.end_tx(first, 6, vtr::TxStatus::Ok).unwrap();
    let second = writer.begin_tx(generator, 7).unwrap();
    writer
        .tx_attr(second, label, &Value::Text("load #1".into()))
        .unwrap();
    writer.end_tx(second, 9, vtr::TxStatus::Ok).unwrap();
    let relation_kind = writer.intern("depends_on");
    writer
        .relate(
            relation_kind,
            first,
            second,
            &[(key, Value::Text("edge".into())), (label, Value::Text("add #0 to load #1".into()))],
        )
        .unwrap();
    writer.close().unwrap();
    verify_labels(&path);
}

fn verify_labels(path: &std::path::Path) {
    let reader = Reader::open(path).unwrap();
    let has_label = |attrs: &[(vtr::StrId, Value)]| {
        attrs.iter().any(|(key, value)| {
            reader.str(*key) == "vtr.label" && matches!(value, Value::Str(_) | Value::Text(_))
        })
    };
    for tx in reader.transactions(&TxQuery::default()).unwrap() {
        assert!(tx.attrs.iter().any(|attr| reader.str(attr.key) == "vtr.label" && matches!(attr.value, Value::Str(_) | Value::Text(_))));
        assert!(tx.events.iter().all(|event| has_label(&event.attrs)));
        assert!(tx.stages.iter().all(|stage| has_label(&stage.attrs)));
    }
    reader
        .visit_relations(|relation| {
            assert!(has_label(&relation.attrs));
            true
        })
        .unwrap();
}
