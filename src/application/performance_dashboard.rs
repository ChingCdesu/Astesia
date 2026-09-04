use super::{PerformanceSnapshot, QueryTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PerformanceRefreshInterval {
    Five,
    Ten,
    Thirty,
    Sixty,
}

impl PerformanceRefreshInterval {
    pub(crate) const ALL: [Self; 4] = [Self::Five, Self::Ten, Self::Thirty, Self::Sixty];

    pub(crate) const fn seconds(self) -> u64 {
        match self {
            Self::Five => 5,
            Self::Ten => 10,
            Self::Thirty => 30,
            Self::Sixty => 60,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PerformanceLoadRequest {
    generation: u64,
    target: QueryTarget,
}

impl PerformanceLoadRequest {
    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PerformanceLoadApply {
    Applied,
    Superseded,
}

pub(crate) struct PerformanceDashboardState {
    target: QueryTarget,
    snapshot: Option<PerformanceSnapshot>,
    error: Option<String>,
    loading: bool,
    available: bool,
    next_generation: u64,
    active_generation: Option<u64>,
}

impl PerformanceDashboardState {
    pub(crate) fn new(target: QueryTarget) -> Self {
        Self {
            target,
            snapshot: None,
            error: None,
            loading: false,
            available: true,
            next_generation: 0,
            active_generation: None,
        }
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }

    pub(crate) fn snapshot(&self) -> Option<&PerformanceSnapshot> {
        self.snapshot.as_ref()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(crate) fn is_available(&self) -> bool {
        self.available
    }

    pub(crate) fn begin_load(&mut self) -> Option<PerformanceLoadRequest> {
        if !self.available || self.loading {
            return None;
        }
        self.next_generation = self.next_generation.saturating_add(1);
        self.active_generation = Some(self.next_generation);
        self.loading = true;
        self.error = None;
        Some(PerformanceLoadRequest {
            generation: self.next_generation,
            target: self.target.clone(),
        })
    }

    pub(crate) fn finish_load(
        &mut self,
        request: PerformanceLoadRequest,
        result: Result<PerformanceSnapshot, String>,
    ) -> PerformanceLoadApply {
        if self.active_generation != Some(request.generation) || self.target != request.target {
            return PerformanceLoadApply::Superseded;
        }
        self.active_generation = None;
        self.loading = false;
        match result {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        PerformanceLoadApply::Applied
    }

    pub(crate) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
    ) -> bool {
        if self.target.connection_id != connection_id
            || self.target.session_generation != session_generation
        {
            return false;
        }
        self.next_generation = self.next_generation.saturating_add(1);
        self.active_generation = None;
        self.loading = false;
        self.available = false;
        self.snapshot = None;
        self.error = Some("Database Session is no longer available".to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::{application::SqliteMetrics, db::DbType};

    use super::*;

    fn target(session_generation: u64) -> QueryTarget {
        QueryTarget {
            connection_id: "sqlite".to_string(),
            connection_name: "Local".to_string(),
            database: "app.sqlite3".to_string(),
            db_type: DbType::SQLite,
            session_generation,
        }
    }

    fn snapshot(page_count: i64) -> PerformanceSnapshot {
        PerformanceSnapshot::SQLite(SqliteMetrics {
            cache_size: -2000,
            page_count,
            page_size: 4096,
            journal_mode: "wal".to_string(),
            wal_pages: 2,
        })
    }

    #[test]
    fn refresh_retains_the_previous_snapshot_and_ignores_superseded_results() {
        let mut state = PerformanceDashboardState::new(target(4));
        let first = state.begin_load().expect("first load");
        assert_eq!(
            state.finish_load(first, Ok(snapshot(12))),
            PerformanceLoadApply::Applied
        );

        let refresh = state.begin_load().expect("refresh");
        assert!(state.is_loading());
        assert_eq!(state.snapshot(), Some(&snapshot(12)));

        state.active_generation = Some(refresh.generation + 1);
        assert_eq!(
            state.finish_load(refresh, Ok(snapshot(18))),
            PerformanceLoadApply::Superseded
        );
        assert_eq!(state.snapshot(), Some(&snapshot(12)));
    }

    #[test]
    fn refresh_failure_keeps_data_visible_and_session_invalidation_stops_loading() {
        let mut state = PerformanceDashboardState::new(target(4));
        let first = state.begin_load().expect("first load");
        state.finish_load(first, Ok(snapshot(12)));

        let refresh = state.begin_load().expect("refresh");
        state.finish_load(refresh, Err("metrics failed".to_string()));
        assert_eq!(state.snapshot(), Some(&snapshot(12)));
        assert_eq!(state.error(), Some("metrics failed"));

        assert!(!state.invalidate_session("sqlite", 3));
        assert!(state.invalidate_session("sqlite", 4));
        assert!(!state.is_available());
        assert!(!state.is_loading());
        assert!(state.snapshot().is_none());
        assert!(state.begin_load().is_none());
    }

    #[test]
    fn refresh_intervals_match_the_milestone_contract() {
        assert_eq!(
            PerformanceRefreshInterval::ALL.map(PerformanceRefreshInterval::seconds),
            [5, 10, 30, 60]
        );
    }
}
