use crate::job::JobType;
use crate::job_type_info::JobTypeInfo;
use crate::load_monitor::{LoadMonitor, LoadMonitorJournalFactory, LoadMonitorStats};
use std::sync::Arc;
use std::time::Duration;

pub trait JobTypeDataEvent: Send + Sync {
    fn notify(&self, duration: Duration);
}

pub trait JobTypeDataCollector: Send + Sync {
    fn make_event(&self, name: &str) -> Arc<dyn JobTypeDataEvent>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobTypeDataEvents {
    pub dequeue: Option<String>,
    pub execute: Option<String>,
}

impl JobTypeDataEvents {
    pub fn new(info: &JobTypeInfo) -> Self {
        if info.special() {
            Self::default()
        } else {
            Self {
                dequeue: Some(format!("{}_q", info.name())),
                execute: Some(info.name().to_owned()),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobTypeDataStats {
    pub average_latency: Duration,
    pub peak_latency: Duration,
    pub load: LoadMonitorStats,
}

pub struct JobTypeData {
    pub info: JobTypeInfo,
    pub waiting: i32,
    pub running: i32,
    pub deferred: i32,
    load: LoadMonitor,
    events: JobTypeDataEvents,
    dequeue_sink: Option<Arc<dyn JobTypeDataEvent>>,
    execute_sink: Option<Arc<dyn JobTypeDataEvent>>,
}

impl std::fmt::Debug for JobTypeData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JobTypeData")
            .field("info", &self.info)
            .field("waiting", &self.waiting)
            .field("running", &self.running)
            .field("deferred", &self.deferred)
            .field("load", &self.load)
            .field("events", &self.events)
            .finish()
    }
}

impl JobTypeData {
    pub fn new(info: JobTypeInfo) -> Self {
        Self::new_with_collector_and_journal_factory::<
            dyn JobTypeDataCollector,
            dyn LoadMonitorJournalFactory,
        >(info, None, None)
    }

    pub fn new_with_collector<C>(info: JobTypeInfo, collector: Option<Arc<C>>) -> Self
    where
        C: JobTypeDataCollector + ?Sized + 'static,
    {
        Self::new_with_collector_and_journal_factory::<C, dyn LoadMonitorJournalFactory>(
            info, collector, None,
        )
    }

    pub fn new_with_collector_and_logs<C, J>(
        info: JobTypeInfo,
        collector: Option<Arc<C>>,
        logs: Option<Arc<J>>,
    ) -> Self
    where
        C: JobTypeDataCollector + ?Sized + 'static,
        J: LoadMonitorJournalFactory + ?Sized + 'static,
    {
        Self::new_with_collector_and_journal_factory(info, collector, logs)
    }

    pub fn new_with_collector_and_journal_factory<C, J>(
        info: JobTypeInfo,
        collector: Option<Arc<C>>,
        journal_factory: Option<Arc<J>>,
    ) -> Self
    where
        C: JobTypeDataCollector + ?Sized + 'static,
        J: LoadMonitorJournalFactory + ?Sized + 'static,
    {
        let load = if let Some(factory) = journal_factory {
            LoadMonitor::with_journal(factory.make_load_monitor_journal("LoadMonitor"))
        } else {
            LoadMonitor::new()
        };
        load.set_target_latency(info.get_average_latency(), info.get_peak_latency());
        let events = JobTypeDataEvents::new(&info);
        let (dequeue_sink, execute_sink) = if info.special() {
            (None, None)
        } else if let Some(collector) = collector {
            (
                events
                    .dequeue
                    .as_deref()
                    .map(|name| collector.make_event(name)),
                events
                    .execute
                    .as_deref()
                    .map(|name| collector.make_event(name)),
            )
        } else {
            (None, None)
        };
        Self {
            info,
            waiting: 0,
            running: 0,
            deferred: 0,
            load,
            events,
            dequeue_sink,
            execute_sink,
        }
    }

    pub fn name(&self) -> &'static str {
        self.info.name()
    }

    pub const fn job_type(&self) -> JobType {
        self.info.job_type()
    }

    pub const fn type_(&self) -> JobType {
        self.info.job_type()
    }

    pub fn load(&self) -> &LoadMonitor {
        &self.load
    }

    pub fn events(&self) -> &JobTypeDataEvents {
        &self.events
    }

    pub fn dequeue_event_name(&self) -> Option<&str> {
        self.events.dequeue.as_deref()
    }

    pub fn execute_event_name(&self) -> Option<&str> {
        self.events.execute.as_deref()
    }

    pub fn notify_dequeue(&self, duration: Duration) {
        if let Some(sink) = &self.dequeue_sink {
            sink.notify(duration);
        }
    }

    pub fn notify_execute(&self, duration: Duration) {
        if let Some(sink) = &self.execute_sink {
            sink.notify(duration);
        }
    }

    pub fn load_stats(&self) -> LoadMonitorStats {
        self.load.get_stats()
    }

    pub fn stats(&self) -> JobTypeDataStats {
        JobTypeDataStats {
            average_latency: self.info.get_average_latency(),
            peak_latency: self.info.get_peak_latency(),
            load: self.load.get_stats(),
        }
    }
}
