//! Simulation log browsing: shared immutable index, cancellable queries and virtual rows.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub stream: Option<u32>,
    pub generator: Option<u32>,
    pub fuzzy: String,
    pub start: String,
    pub end: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationLogsTile {
    pub query: Query,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    runtime: std::cell::RefCell<native::Runtime>,
}
impl Clone for SimulationLogsTile {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            ..Default::default()
        }
    }
}
impl SimulationLogsTile {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn ready(&self) -> bool {
        self.runtime.borrow().ready()
    }
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn rendered_rows(&self) -> usize {
        self.runtime.borrow().rendered_rows.get()
    }
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn indexed_rows(&self) -> usize {
        self.runtime.borrow().indexed_rows()
    }
    pub(crate) fn ui(&self, ui: &mut egui::Ui, cx: &mut crate::tiles::view::TileCtx<'_>) {
        #[cfg(not(target_arch = "wasm32"))]
        native::draw(self, ui, cx);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = cx;
            ui.label("Simulation logs are available in native Surfer.");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native {
    use super::*;
    use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
    use std::sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use vtr::{LogQuery, Reader};

    pub struct Generator {
        pub id: u32,
        pub stream: u32,
        pub label: String,
    }
    pub struct Source {
        reader: Arc<Mutex<Reader>>,
        pub streams: Vec<(u32, String)>,
        pub generators: Vec<Generator>,
        pub timescale: i8,
        index: OnceLock<Result<Arc<Index>, String>>,
    }
    pub struct Row {
        pub time: u64,
        pub id: u64,
        pub generator: u32,
        pub stream: u32,
        pub severity: vtr::Severity,
        pub text: String,
    }
    pub struct Index {
        pub rows: Vec<Row>,
    }
    impl Source {
        pub fn new(reader: Arc<Mutex<Reader>>) -> Arc<Self> {
            let r = reader.lock().unwrap();
            let mut streams = std::collections::BTreeMap::new();
            let generators = r
                .log_sites()
                .iter()
                .map(|site| {
                    streams
                        .entry(site.stream.0)
                        .or_insert_with(|| r.str(r.hierarchy().name(site.stream)).to_owned());
                    Generator {
                        id: site.node.0,
                        stream: site.stream.0,
                        label: if r.str(site.fmt) == "{}" {
                            site.severity.name().to_owned()
                        } else {
                            format!("{} · {}", site.severity.name(), r.str(site.fmt))
                        },
                    }
                })
                .collect();
            let timescale = r.meta().timescale;
            drop(r);
            Arc::new(Self {
                reader,
                streams: streams.into_iter().collect(),
                generators,
                timescale,
                index: OnceLock::new(),
            })
        }
        fn index(&self) -> Result<Arc<Index>, String> {
            self.index
                .get_or_init(|| {
                    let mut reader = self.reader.lock().map_err(|e| e.to_string())?;
                    let mut rows = Vec::new();
                    let result = reader
                        .visit_log(&LogQuery::default(), |record| {
                            rows.push(Row {
                                time: record.time,
                                id: record.id,
                                generator: record.site.node.0,
                                stream: record.site.stream.0,
                                severity: record.severity(),
                                text: record.format(reader.strings()),
                            });
                            true
                        })
                        .map_err(|e| e.to_string());
                    reader.clear_cache();
                    result?;
                    rows.sort_unstable_by_key(|r| (r.time, r.id));
                    Ok(Arc::new(Index { rows }))
                })
                .clone()
        }
    }
    pub struct Results {
        pub index: Arc<Index>,
        pub matches: Vec<usize>,
    }
    fn query(index: Arc<Index>, q: &Query, cancel: &AtomicBool) -> Result<Results, String> {
        let parse = |s: &str, default| {
            if s.trim().is_empty() {
                Ok(default)
            } else {
                s.trim()
                    .parse::<u64>()
                    .map_err(|_| "Time must be an unsigned integer in recording ticks".to_owned())
            }
        };
        let start = parse(&q.start, 0)?;
        let end = parse(&q.end, u64::MAX)?;
        if start > end {
            return Err("Start time must not exceed end time".into());
        }
        let lo = index.rows.partition_point(|r| r.time < start);
        let hi = index.rows.partition_point(|r| r.time <= end);
        let matcher = SkimMatcherV2::default().ignore_case();
        let mut matches = Vec::new();
        for i in lo..hi {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let row = &index.rows[i];
            if q.stream.is_some_and(|id| row.stream != id)
                || q.generator.is_some_and(|id| row.generator != id)
            {
                continue;
            }
            if q.fuzzy.is_empty() || matcher.fuzzy_match(&row.text, &q.fuzzy).is_some() {
                matches.push(i);
            }
        }
        Ok(Results { index, matches })
    }
    struct Job {
        cancel: Arc<AtomicBool>,
        rx: mpsc::Receiver<Result<Results, String>>,
        query: Query,
    }
    #[derive(Default)]
    pub struct Runtime {
        reset_scroll: bool,
        #[cfg(test)]
        pub(super) rendered_rows: std::cell::Cell<usize>,
        source: Option<Arc<Source>>,
        job: Option<Job>,
        requested: Option<Query>,
        result: Option<Result<Results, String>>,
    }
    impl Drop for Runtime {
        fn drop(&mut self) {
            if let Some(job) = &self.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
    impl Runtime {
        #[cfg(test)]
        pub(super) fn ready(&self) -> bool {
            matches!(self.result, Some(Ok(_)))
        }
        #[cfg(test)]
        pub(super) fn indexed_rows(&self) -> usize {
            self.result
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .index
                .rows
                .len()
        }
        fn update(&mut self, source: &Arc<Source>, q: &Query, ctx: &egui::Context) {
            if self
                .source
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, source))
            {
                *self = Self::default();
                self.source = Some(source.clone());
            }
            if self.requested.as_ref() != Some(q) {
                if let Some(job) = &self.job {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                self.requested = Some(q.clone());
                self.reset_scroll = true;
                self.result = None;
            }
            if let Some(job) = &self.job {
                match job.rx.try_recv() {
                    Ok(result) => {
                        if job.query == *q && !job.cancel.load(Ordering::Relaxed) {
                            self.result = Some(result);
                        }
                        self.job = None;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.result = Some(Err("Log worker stopped unexpectedly".into()));
                        self.job = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if self.job.is_none() && self.result.is_none() {
                let (tx, rx) = mpsc::channel();
                let cancel = Arc::new(AtomicBool::new(false));
                self.job = Some(Job {
                    cancel: cancel.clone(),
                    rx,
                    query: q.clone(),
                });
                let source = source.clone();
                let q = q.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = source.index().and_then(|index| query(index, &q, &cancel));
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
            }
        }
    }
    pub fn draw(
        tile: &SimulationLogsTile,
        ui: &mut egui::Ui,
        cx: &mut crate::tiles::view::TileCtx<'_>,
    ) {
        let source = cx
            .waves()
            .and_then(|w| w.inner.as_transactions())
            .and_then(|t| t.native.as_ref())
            .map(|n| n.logs.clone());
        let Some(source) = source.filter(|s| !s.streams.is_empty()) else {
            *tile.runtime.borrow_mut() = Runtime::default();
            ui.centered_and_justified(|ui| {
                ui.label("Open a VTR recording with log streams to browse simulation messages.");
            });
            return;
        };
        let mut q = tile.query.clone();
        if q.stream
            .is_none_or(|id| !source.streams.iter().any(|s| s.0 == id))
        {
            q.stream = Some(source.streams[0].0);
            q.generator = None;
        }
        if q.generator.is_some_and(|id| {
            !source
                .generators
                .iter()
                .any(|g| g.id == id && Some(g.stream) == q.stream)
        }) {
            q.generator = None;
        }
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(cx.id("log_stream"))
                .selected_text(
                    source
                        .streams
                        .iter()
                        .find(|s| Some(s.0) == q.stream)
                        .map(|s| s.1.as_str())
                        .unwrap_or("Stream"),
                )
                .show_ui(ui, |ui| {
                    for (id, name) in &source.streams {
                        if ui
                            .selectable_value(&mut q.stream, Some(*id), name)
                            .changed()
                        {
                            q.generator = None;
                        }
                    }
                });
            egui::ComboBox::from_id_salt(cx.id("log_generator"))
                .selected_text(
                    source
                        .generators
                        .iter()
                        .find(|g| Some(g.id) == q.generator)
                        .map(|g| g.label.as_str())
                        .unwrap_or("All generators"),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut q.generator, None, "All generators");
                    for g in &source.generators {
                        if Some(g.stream) == q.stream {
                            ui.selectable_value(&mut q.generator, Some(g.id), &g.label);
                        }
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut q.fuzzy)
                    .hint_text("Fuzzy search messages…")
                    .desired_width(230.0),
            );
        });
        let time_unit = match crate::vtr_adapter::timescale(source.timescale) {
            Ok(scale) => {
                let unit = crate::time::TimeUnit::from(scale.unit);
                if scale.factor == 1 {
                    unit.to_string()
                } else {
                    format!("{} {unit}", scale.factor)
                }
            }
            Err(_) => format!("10^{} s", source.timescale),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Time ({time_unit})"));
            ui.add(
                egui::TextEdit::singleline(&mut q.start)
                    .hint_text("From")
                    .desired_width(95.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut q.end)
                    .hint_text("To")
                    .desired_width(95.0),
            );
            if ui.button("Clear filters").clicked() {
                q = Query {
                    stream: q.stream,
                    ..Default::default()
                };
            }
        });
        if q != tile.query {
            cx.send_self(crate::tiles::kind::TileMessage::SimulationLogs(q.clone()));
        }
        let mut runtime = tile.runtime.borrow_mut();
        runtime.update(&source, &q, ui.ctx());
        let reset_scroll = runtime.result.is_some() && std::mem::take(&mut runtime.reset_scroll);
        ui.separator();
        let Some(result) = &runtime.result else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching simulation logs…");
            });
            return;
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                return;
            }
        };
        ui.label(format!("{} messages", result.matches.len()));
        ui.horizontal(|ui| {
            ui.add_sized([130.0, 20.0], egui::Label::new(format!("TIME ({time_unit})")));
            ui.add_sized([75.0, 20.0], egui::Label::new("SEVERITY"));
            ui.label("MESSAGE");
        });
        let scroll = egui::ScrollArea::both().id_salt(cx.id("simulation_log_rows"));
        let scroll = if reset_scroll {
            scroll.vertical_scroll_offset(0.0)
        } else {
            scroll
        };
        scroll.show_rows(ui, 22.0, result.matches.len(), |ui, range| {
            #[cfg(test)]
            runtime.rendered_rows.set(range.len());
            for i in range {
                let row = &result.index.rows[result.matches[i]];
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            [130.0, 22.0],
                            egui::Button::new(
                                egui::RichText::new(row.time.to_string()).monospace(),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Set waveform cursor to this time")
                        .clicked()
                    {
                        cx.send_document(crate::tiles::commands::DocumentCommand::CursorSet(
                            row.time.into(),
                        ));
                    }
                    let color = match row.severity {
                        vtr::Severity::Fatal | vtr::Severity::Error => {
                            egui::Color32::from_rgb(247, 115, 115)
                        }
                        vtr::Severity::Warn => egui::Color32::from_rgb(236, 186, 90),
                        _ => ui.visuals().text_color(),
                    };
                    ui.add_sized(
                        [75.0, 22.0],
                        egui::Label::new(egui::RichText::new(row.severity.name()).color(color)),
                    );
                    let text = row.text.trim_end().replace('\n', " ↵ ");
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace()).extend())
                        .on_hover_text(&row.text)
                        .context_menu(|ui| {
                            if ui.button("Copy message").clicked() {
                                ui.ctx().copy_text(row.text.clone());
                                ui.close();
                            }
                        });
                });
            }
        });
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        fn source() -> Arc<Source> {
            Source::new(Arc::new(Mutex::new(
                Reader::open("../examples/simulation_logs.vtr").unwrap(),
            )))
        }
        #[test]
        fn indexed_query_preserves_time_order_and_filters_stream_generator_and_fuzzy_text() {
            let source = source();
            let index = source.index().unwrap();
            assert!(Arc::ptr_eq(&index, &source.index().unwrap()));
            assert_eq!(index.rows.len(), 2020);
            let stream = source
                .streams
                .iter()
                .find(|s| s.1 == "simulation_log")
                .unwrap()
                .0;
            let q = Query {
                stream: Some(stream),
                fuzzy: "dmch".into(),
                start: "0".into(),
                end: "200".into(),
                ..Default::default()
            };
            let result = query(index.clone(), &q, &AtomicBool::new(false)).unwrap();
            let times: Vec<_> = result
                .matches
                .iter()
                .map(|&i| result.index.rows[i].time)
                .collect();
            assert_eq!(times, [20, 90, 100, 130, 200]);
            let generator = result.index.rows[result.matches[0]].generator;
            let q = Query {
                generator: Some(generator),
                fuzzy: String::new(),
                ..q
            };
            assert_eq!(
                query(index.clone(), &q, &AtomicBool::new(false))
                    .unwrap()
                    .matches
                    .len(),
                4
            );
            assert!(query(index.clone(), &q, &AtomicBool::new(true)).is_err());
            assert!(
                query(
                    index.clone(),
                    &Query {
                        start: "x".into(),
                        ..Default::default()
                    },
                    &AtomicBool::new(false)
                )
                .is_err()
            );
            assert!(
                query(
                    index,
                    &Query {
                        start: "2".into(),
                        end: "1".into(),
                        ..Default::default()
                    },
                    &AtomicBool::new(false)
                )
                .is_err()
            );
        }
        #[test]
        fn replacement_and_rapid_queries_never_publish_stale_results() {
            let source = source();
            let context = egui::Context::default();
            let mut runtime = Runtime::default();
            let q = Query {
                fuzzy: "will not match anything".into(),
                ..Default::default()
            };
            runtime.update(&source, &Query::default(), &context);
            runtime.update(&source, &q, &context);
            let start = std::time::Instant::now();
            while !runtime.ready() {
                runtime.update(&source, &q, &context);
                assert!(start.elapsed().as_secs() < 10);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(
                runtime
                    .result
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .matches
                    .is_empty()
            );
            let other = super::tests::source();
            runtime.update(&other, &Query::default(), &context);
            assert!(runtime.result.is_none());
            assert!(Arc::ptr_eq(runtime.source.as_ref().unwrap(), &other));
        }
    }
}
