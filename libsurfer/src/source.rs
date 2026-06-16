use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use eyre::{Result, bail};
use num::BigUint;
use num::Zero as _;
use serde::{Deserialize, Serialize};

use crate::analog_signal_cache::AnalogCacheEntry;
use crate::data_container::DataContainer;
use crate::time::{TimeScale, TimeUnit};
use crate::transaction_container::{
    StreamScopeRef, TransactionContainer, TransactionRef, TransactionStreamRef,
};
use crate::wave_container::{AnalogCacheKey, ScopeRef, VariableRef, WaveContainer};
use crate::wave_source::{WaveFormat, WaveSource};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SourceId(pub u64);

impl Display for SourceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "source {}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LoadRequestId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceScopeRef<T> {
    #[serde(default)]
    pub source: SourceId,
    #[serde(flatten)]
    pub inner: T,
}

impl<T> SourceScopeRef<T> {
    #[must_use]
    pub fn new(source: SourceId, inner: T) -> Self {
        Self { source, inner }
    }

    #[must_use]
    pub fn primary(inner: T) -> Self {
        Self::new(SourceId::default(), inner)
    }
}

pub type SourceVariableRef = SourceScopeRef<VariableRef>;
pub type SourceWaveScopeRef = SourceScopeRef<ScopeRef>;
pub type SourceStreamScopeRef = SourceScopeRef<StreamScopeRef>;
pub type SourceTransactionStreamRef = SourceScopeRef<TransactionStreamRef>;
pub type SourceTransactionRef = SourceScopeRef<TransactionRef>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActiveScope {
    Wave(SourceWaveScopeRef),
    Stream(SourceStreamScopeRef),
}

impl ActiveScope {
    #[must_use]
    pub fn source(&self) -> SourceId {
        match self {
            ActiveScope::Wave(scope) => scope.source,
            ActiveScope::Stream(scope) => scope.source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeDomain {
    pub timescale: TimeScale,
    pub max_timestamp: BigUint,
}

impl TimeDomain {
    #[must_use]
    pub fn from_container(inner: &DataContainer) -> Option<Self> {
        let max_timestamp = inner.max_timestamp()?;
        let mut timescale = inner.metadata().timescale;
        if timescale.unit != TimeUnit::None && timescale.multiplier.is_none() {
            timescale.multiplier = Some(1);
        }
        Some(Self {
            timescale,
            max_timestamp,
        })
    }

    #[must_use]
    pub fn normalized_for_viewport(&self) -> Option<BigUint> {
        (!self.max_timestamp.is_zero()).then(|| self.max_timestamp.clone())
    }

    #[must_use]
    pub fn has_same_timescale(&self, other: &Self) -> bool {
        self.timescale == other.timescale
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceLoadState {
    #[default]
    Loaded,
    Pending,
    Error(String),
}

#[derive(Serialize, Deserialize)]
pub struct LoadedSource {
    pub id: SourceId,
    pub label: String,
    pub accent_color: Option<String>,
    pub source: WaveSource,
    pub format: WaveFormat,
    #[serde(skip, default = "DataContainer::__new_empty")]
    pub inner: DataContainer,
    pub time_domain: Option<TimeDomain>,
    pub selected_server_file_index: Option<usize>,
    pub cache_generation: u64,
    #[serde(skip, default)]
    pub inflight_caches: HashMap<AnalogCacheKey, Arc<AnalogCacheEntry>>,
    #[serde(default)]
    pub load_state: SourceLoadState,
    #[serde(skip)]
    pub active_load_request: Option<LoadRequestId>,
}

impl LoadedSource {
    #[must_use]
    pub fn new(id: SourceId, source: WaveSource, format: WaveFormat, inner: DataContainer) -> Self {
        let label = source_label(&source);
        let time_domain = TimeDomain::from_container(&inner);
        Self {
            id,
            label,
            accent_color: None,
            source,
            format,
            inner,
            time_domain,
            selected_server_file_index: None,
            cache_generation: 0,
            inflight_caches: HashMap::new(),
            load_state: SourceLoadState::Loaded,
            active_load_request: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SourceStore {
    pub sources: Vec<LoadedSource>,
    pub next_source_id: u64,
    pub session_time_domain: Option<TimeDomain>,
}

impl Default for SourceStore {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            next_source_id: 1,
            session_time_domain: None,
        }
    }
}

impl SourceStore {
    #[must_use]
    pub fn with_primary_domain(inner: &DataContainer) -> Self {
        Self {
            sources: Vec::new(),
            next_source_id: 1,
            session_time_domain: TimeDomain::from_container(inner),
        }
    }

    #[must_use]
    pub fn source(&self, id: SourceId) -> Option<&LoadedSource> {
        self.sources.iter().find(|source| source.id == id)
    }

    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut LoadedSource> {
        self.sources.iter_mut().find(|source| source.id == id)
    }

    #[must_use]
    pub fn waves(&self, id: SourceId) -> Option<&WaveContainer> {
        self.source(id)?.inner.as_waves()
    }

    pub fn waves_mut(&mut self, id: SourceId) -> Option<&mut WaveContainer> {
        self.source_mut(id)?.inner.as_waves_mut()
    }

    #[must_use]
    pub fn transactions(&self, id: SourceId) -> Option<&TransactionContainer> {
        self.source(id)?.inner.as_transactions()
    }

    pub fn transactions_mut(&mut self, id: SourceId) -> Option<&mut TransactionContainer> {
        self.source_mut(id)?.inner.as_transactions_mut()
    }

    #[must_use]
    pub fn common_time_domain(&self) -> Option<&TimeDomain> {
        self.session_time_domain.as_ref()
    }

    pub fn validate_time_domain(&self, candidate: &TimeDomain) -> Result<()> {
        match &self.session_time_domain {
            None => Ok(()),
            Some(existing) if existing.has_same_timescale(candidate) => Ok(()),
            Some(existing) => {
                bail!(
                    "Time scale differs: existing {}, candidate {}",
                    format_time_domain(existing),
                    format_time_domain(candidate)
                )
            }
        }
    }

    pub fn merge_time_domain(&mut self, candidate: TimeDomain) {
        match &mut self.session_time_domain {
            Some(existing) => {
                if candidate.max_timestamp > existing.max_timestamp {
                    existing.max_timestamp = candidate.max_timestamp;
                }
            }
            None => self.session_time_domain = Some(candidate),
        }
    }

    pub fn recompute_session_time_domain(
        &mut self,
        primary_domain: Option<TimeDomain>,
    ) -> Result<()> {
        let mut domains = primary_domain
            .into_iter()
            .chain(self.sources.iter().filter_map(|source| {
                source
                    .time_domain
                    .clone()
                    .or_else(|| TimeDomain::from_container(&source.inner))
            }));

        let Some(mut session_domain) = domains.next() else {
            self.session_time_domain = None;
            return Ok(());
        };

        for domain in domains {
            if !session_domain.has_same_timescale(&domain) {
                bail!(
                    "Time scale differs: existing {}, candidate {}",
                    format_time_domain(&session_domain),
                    format_time_domain(&domain)
                );
            }
            if domain.max_timestamp > session_domain.max_timestamp {
                session_domain.max_timestamp = domain.max_timestamp;
            }
        }

        self.session_time_domain = Some(session_domain);
        Ok(())
    }

    pub fn add_source(
        &mut self,
        source: WaveSource,
        format: WaveFormat,
        inner: DataContainer,
    ) -> Result<SourceId> {
        let candidate_domain = TimeDomain::from_container(&inner);
        if let Some(candidate_domain) = &candidate_domain {
            self.validate_time_domain(candidate_domain)?;
        }
        if let Some(candidate_domain) = candidate_domain {
            self.merge_time_domain(candidate_domain);
        }

        let id = SourceId(self.next_source_id);
        self.next_source_id += 1;
        self.sources
            .push(LoadedSource::new(id, source, format, inner));
        Ok(id)
    }

    pub fn add_pending_source(
        &mut self,
        source: WaveSource,
        format: WaveFormat,
        inner: DataContainer,
        request: LoadRequestId,
    ) -> SourceId {
        let id = SourceId(self.next_source_id);
        self.next_source_id += 1;
        let mut loaded_source = LoadedSource::new(id, source, format, inner);
        loaded_source.load_state = SourceLoadState::Pending;
        loaded_source.active_load_request = Some(request);
        self.sources.push(loaded_source);
        id
    }

    pub fn remove_source(&mut self, id: SourceId) -> Option<LoadedSource> {
        let idx = self.sources.iter().position(|source| source.id == id)?;
        Some(self.sources.remove(idx))
    }
}

#[must_use]
pub fn format_time_domain(domain: &TimeDomain) -> String {
    format!(
        "{} {}, 0..{}",
        domain.timescale.multiplier.unwrap_or(1),
        domain.timescale.unit,
        domain.max_timestamp
    )
}

#[must_use]
pub fn source_label(source: &WaveSource) -> String {
    match source {
        WaveSource::File(path) => path
            .file_name()
            .map(ToString::to_string)
            .unwrap_or_else(|| path.to_string()),
        WaveSource::Url(url) => url.clone(),
        WaveSource::Data => "File data".to_string(),
        WaveSource::DragAndDrop(Some(path)) => path
            .file_name()
            .map(|name| format!("Dropped file ({name})"))
            .unwrap_or_else(|| "Dropped file".to_string()),
        WaveSource::DragAndDrop(None) => "Dropped file".to_string(),
        WaveSource::Cxxrtl(_) => "CXXRTL".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::TimeUnit;

    fn domain(unit: TimeUnit, multiplier: Option<u32>, max_timestamp: u64) -> TimeDomain {
        TimeDomain {
            timescale: TimeScale { unit, multiplier },
            max_timestamp: max_timestamp.into(),
        }
    }

    #[test]
    fn matching_time_domain_is_accepted() {
        let store = SourceStore {
            session_time_domain: Some(domain(TimeUnit::NanoSeconds, Some(1), 100)),
            ..Default::default()
        };

        assert!(
            store
                .validate_time_domain(&domain(TimeUnit::NanoSeconds, Some(1), 100))
                .is_ok()
        );
    }

    #[test]
    fn different_time_unit_is_rejected() {
        let store = SourceStore {
            session_time_domain: Some(domain(TimeUnit::NanoSeconds, Some(1), 100)),
            ..Default::default()
        };

        assert!(
            store
                .validate_time_domain(&domain(TimeUnit::PicoSeconds, Some(1), 100))
                .is_err()
        );
    }

    #[test]
    fn different_multiplier_is_rejected() {
        let store = SourceStore {
            session_time_domain: Some(domain(TimeUnit::NanoSeconds, Some(1), 100)),
            ..Default::default()
        };

        assert!(
            store
                .validate_time_domain(&domain(TimeUnit::NanoSeconds, Some(10), 100))
                .is_err()
        );
    }

    #[test]
    fn longer_matching_time_domain_extends_session_span() {
        let mut store = SourceStore {
            session_time_domain: Some(domain(TimeUnit::NanoSeconds, Some(1), 100)),
            ..Default::default()
        };

        let candidate = domain(TimeUnit::NanoSeconds, Some(1), 101);
        assert!(store.validate_time_domain(&candidate).is_ok());

        store.merge_time_domain(candidate);
        assert_eq!(
            store.session_time_domain,
            Some(domain(TimeUnit::NanoSeconds, Some(1), 101))
        );
    }

    #[test]
    fn shorter_matching_time_domain_keeps_session_span() {
        let mut store = SourceStore {
            session_time_domain: Some(domain(TimeUnit::NanoSeconds, Some(1), 100)),
            ..Default::default()
        };

        let candidate = domain(TimeUnit::NanoSeconds, Some(1), 99);
        assert!(store.validate_time_domain(&candidate).is_ok());

        store.merge_time_domain(candidate);
        assert_eq!(
            store.session_time_domain,
            Some(domain(TimeUnit::NanoSeconds, Some(1), 100))
        );
    }

    #[test]
    fn time_unit_none_only_matches_none() {
        let store = SourceStore {
            session_time_domain: Some(domain(TimeUnit::None, None, 100)),
            ..Default::default()
        };

        assert!(
            store
                .validate_time_domain(&domain(TimeUnit::None, None, 100))
                .is_ok()
        );
        assert!(
            store
                .validate_time_domain(&domain(TimeUnit::Seconds, None, 100))
                .is_err()
        );
    }
}
