use crate::job::JobType;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobTypeInfo {
    job_type: JobType,
    name: &'static str,
    limit: i32,
    average_latency: Duration,
    peak_latency: Duration,
}

impl JobTypeInfo {
    pub const fn new(
        job_type: JobType,
        name: &'static str,
        limit: i32,
        average_latency: Duration,
        peak_latency: Duration,
    ) -> Self {
        Self {
            job_type,
            name,
            limit,
            average_latency,
            peak_latency,
        }
    }

    pub const fn job_type(&self) -> JobType {
        self.job_type
    }

    pub const fn type_(&self) -> JobType {
        self.job_type
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn limit(&self) -> i32 {
        self.limit
    }

    pub const fn special(&self) -> bool {
        self.limit == 0
    }

    pub const fn average_latency(&self) -> Duration {
        self.average_latency
    }

    pub const fn get_average_latency(&self) -> Duration {
        self.average_latency
    }

    pub const fn peak_latency(&self) -> Duration {
        self.peak_latency
    }

    pub const fn get_peak_latency(&self) -> Duration {
        self.peak_latency
    }
}
