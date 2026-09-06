//! Geometry from the VDB netlist and the pinned native ELK engine.
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use serde_json::{Value, json};
use vtr_vdb::{
    Database,
    netlist::{Netlist, NetlistIndex},
};

pub(super) struct Node {
    pub rect: Rect,
    pub pins: Vec<Pos2>,
}
pub(super) struct Wire {
    pub points: Vec<Pos2>,
    pub symbol: Option<String>,
    pub source_block: usize,
}
pub(super) struct Scene {
    pub netlist: Netlist,
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    pub bounds: Rect,
}

pub(super) fn signal(block: &vtr_vdb::netlist::Block) -> Option<&str> {
    (block.detail.starts_with("signal")
        || block.detail.starts_with("external reference")
        || block.detail.contains("port ·"))
    .then(|| block.pins.iter().find_map(|p| p.symbol.as_deref()))
    .flatten()
}

impl Scene {
    pub fn build(db: &Database, instance: &str) -> Result<Self, String> {
        let netlist = NetlistIndex::new(db)?.module(instance)?;
        let children: Vec<_> = netlist.blocks.iter().map(|block| {
            let is_signal = signal(block).is_some();
            let width = if is_signal { 128.0 } else if block.child.is_some() { 224.0 } else { 170.0 };
            let pin_count = block.pins.iter().filter(|p| p.output).count().max(block.pins.iter().filter(|p| !p.output).count());
            let height = if is_signal { 42.0 } else { 54.0 + pin_count as f32 * 23.0 };
            let mut sides = [0, 0];
            let ports: Vec<_> = block.pins.iter().enumerate().map(|(i, pin)| {
                let side = usize::from(pin.output);
                let y = if is_signal { height * 0.5 } else { 54.0 + sides[side] as f32 * 23.0 };
                sides[side] += 1;
                json!({"id":format!("{}p{i}", block.id), "x":if pin.output {width} else {0.0}, "y":y,
                    "width":0,"height":0,"layoutOptions":{"elk.port.side":if pin.output {"EAST"} else {"WEST"}}})
            }).collect();
            json!({"id":block.id,"width":width,"height":height,"ports":ports,"layoutOptions":{"elk.portConstraints":"FIXED_POS"}})
        }).collect();
        let edges: Vec<_> = netlist.wires.iter().enumerate().map(|(i, wire)| json!({"id":format!("e{i}"),
            "sources":[format!("n{}p{}",wire.source.0,wire.source.1)],"targets":[format!("n{}p{}",wire.target.0,wire.target.1)]})).collect();
        let graph = json!({"id":"root","children":children,"edges":edges,"layoutOptions":{
            "elk.algorithm":"layered","elk.direction":"RIGHT","elk.edgeRouting":"ORTHOGONAL",
            "elk.spacing.nodeNode":"30","elk.layered.spacing.nodeNodeBetweenLayers":"72","elk.spacing.edgeNode":"18",
            "elk.layered.spacing.edgeNodeBetweenLayers":"18","elk.randomSeed":"1"}});
        let layout = elkrs::create_elk()
            .layout_json(&graph.to_string())
            .map_err(|e| format!("Layout failed: {e}"))?;
        let empty = vec![];
        let json_nodes = layout["children"].as_array().unwrap_or(&empty);
        let mut nodes = Vec::new();
        for block in &netlist.blocks {
            let node = json_nodes
                .iter()
                .find(|n| n["id"].as_str() == Some(&block.id))
                .ok_or("Layout omitted a block")?;
            let origin = point(node)?;
            let size = vec2(number(node, "width")?, number(node, "height")?);
            let mut pins = Vec::new();
            for i in 0..block.pins.len() {
                let id = format!("{}p{i}", block.id);
                let pin = node["ports"]
                    .as_array()
                    .and_then(|ports| ports.iter().find(|p| p["id"].as_str() == Some(&id)))
                    .ok_or("Layout omitted a pin")?;
                pins.push(origin + point(pin)?.to_vec2());
            }
            nodes.push(Node {
                rect: Rect::from_min_size(origin, size),
                pins,
            });
        }
        let json_edges = layout["edges"].as_array().unwrap_or(&empty);
        let mut wires = Vec::new();
        for (i, wire) in netlist.wires.iter().enumerate() {
            let id = format!("e{i}");
            let edge = json_edges
                .iter()
                .find(|e| e["id"].as_str() == Some(&id))
                .ok_or("Layout omitted a wire")?;
            let sections = edge["sections"]
                .as_array()
                .ok_or("Layout omitted wire routing")?;
            let source = &netlist.blocks[wire.source.0];
            let target = &netlist.blocks[wire.target.0];
            let candidates = [
                signal(source),
                signal(target),
                source.pins[wire.source.1].symbol.as_deref(),
                target.pins[wire.target.1].symbol.as_deref(),
            ];
            let symbol = candidates
                .iter()
                .flatten()
                .find(|s| db.symbols.get(**s).is_some_and(|s| s.owner == instance))
                .copied()
                .or_else(|| candidates.into_iter().flatten().next())
                .map(str::to_owned);
            for section in sections {
                let mut points = vec![point(&section["startPoint"])?];
                if let Some(bends) = section["bendPoints"].as_array() {
                    for bend in bends {
                        points.push(point(bend)?);
                    }
                }
                points.push(point(&section["endPoint"])?);
                wires.push(Wire {
                    points,
                    symbol: symbol.clone(),
                    source_block: wire.source.0,
                });
            }
        }
        let mut bounds = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0));
        for node in &nodes {
            bounds = bounds.union(node.rect);
        }
        for wire in &wires {
            for point in &wire.points {
                bounds.extend_with(*point);
            }
        }
        Ok(Self {
            netlist,
            nodes,
            wires,
            bounds: bounds.expand(24.0),
        })
    }
}
fn number(value: &Value, key: &str) -> Result<f32, String> {
    value[key]
        .as_f64()
        .filter(|v| v.is_finite() && v.abs() < 1e8)
        .map(|v| v as f32)
        .ok_or_else(|| format!("Invalid layout coordinate: {key}"))
}
fn point(value: &Value) -> Result<Pos2, String> {
    Ok(pos2(number(value, "x")?, number(value, "y")?))
}
