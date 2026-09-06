//! Reproduce the supplemental transaction examples (not simulator waveforms).
use vtr::{
    AttrPhase, Direction as VtrDirection, ScopeType as VtrScopeType, ScopeType, SignalKind, Value,
    VarType as VtrVarType, Writer,
};
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    combined(root.join("combined.vtr"));
    transactions(root.join("transactions.vtr"));
}
fn combined(path: std::path::PathBuf) {
    let mut writer = Writer::create(&path).unwrap();
    writer.begin_scope("top", VtrScopeType::Module, "top");
    let (_, count) = writer.add_var(
        "count",
        VtrVarType::Logic,
        VtrDirection::Output,
        SignalKind::Bits {
            width: 8,
            states: 4,
        },
    );
    writer.end_scope().unwrap();
    let stream = writer.add_stream(None, "cpu", "PIPELINE");
    let generator = writer.add_generator(stream, "issue");
    let key = writer.intern("opcode");
    let event_name = writer.intern("retire");
    let stage_name = writer.intern("execute");
    let lane_name = writer.intern("alu");
    writer.set_time(0).unwrap();
    writer.emit_u64(count, 3).unwrap();
    writer.set_time(4).unwrap();
    writer.emit_u64(count, 7).unwrap();
    let tx = writer.begin_tx(generator, 1).unwrap();
    writer.set_tx_kind(tx, vtr::TxKind::Internal).unwrap();
    writer.tx_event(tx, 2, event_name, &[]).unwrap();
    writer
        .tx_stage(tx, stage_name, lane_name, 1, 2, &[])
        .unwrap();
    writer
        .tx_attr(tx, key, AttrPhase::Record, &vtr::Value::Text("add".into()))
        .unwrap();
    writer.end_tx(tx, 3, vtr::TxStatus::Ok).unwrap();
    writer.close().unwrap();
}
fn transactions(path: std::path::PathBuf) {
    let mut writer = Writer::create(&path).unwrap();
    let stream = writer.add_stream(None, "cpu", "PIPELINE");
    let generator = writer.add_generator(stream, "issue");
    writer.begin_scope("top", ScopeType::Module, "top");
    writer.end_scope().unwrap();
    let key = writer.intern("opcode");
    let event_name = writer.intern("retire");
    let stage_name = writer.intern("execute");
    let lane_name = writer.intern("alu");
    let first = writer.begin_tx(generator, 2).unwrap();
    writer.set_tx_kind(first, vtr::TxKind::Internal).unwrap();
    writer.tx_event(first, 3, event_name, &[]).unwrap();
    writer
        .tx_stage(first, stage_name, lane_name, 2, 5, &[])
        .unwrap();
    writer
        .tx_attr(first, key, AttrPhase::Record, &Value::Text("add".into()))
        .unwrap();
    writer.end_tx(first, 6, vtr::TxStatus::Ok).unwrap();
    let second = writer.begin_tx(generator, 7).unwrap();
    writer.end_tx(second, 9, vtr::TxStatus::Ok).unwrap();
    let relation_kind = writer.intern("depends_on");
    writer
        .relate(
            relation_kind,
            first,
            second,
            &[(key, Value::Text("edge".into()))],
        )
        .unwrap();
    writer.close().unwrap();
}
