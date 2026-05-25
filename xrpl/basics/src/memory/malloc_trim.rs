//! Compatibility surface for `xrpl/basics/MallocTrim.h`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MallocTrimReport {
    pub supported: bool,
    pub trim_result: i32,
    pub rss_before_kb: i64,
    pub rss_after_kb: i64,
    pub duration_us: i64,
    pub minflt_delta: i64,
    pub majflt_delta: i64,
}

impl Default for MallocTrimReport {
    fn default() -> Self {
        Self {
            supported: false,
            trim_result: -1,
            rss_before_kb: -1,
            rss_after_kb: -1,
            duration_us: -1,
            minflt_delta: -1,
            majflt_delta: -1,
        }
    }
}

impl MallocTrimReport {
    pub fn delta_kb(&self) -> i64 {
        if self.rss_before_kb < 0 || self.rss_after_kb < 0 {
            return 0;
        }
        self.rss_after_kb - self.rss_before_kb
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NullMallocTrimLogger;

pub trait MallocTrimLogger: Send + Sync + 'static {
    fn debug(&self, message: &str);
}

impl MallocTrimLogger for NullMallocTrimLogger {
    fn debug(&self, _message: &str) {}
}

pub fn malloc_trim(tag: &str, logger: &dyn MallocTrimLogger) -> MallocTrimReport {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let rss_before_kb = read_statm_rss_kb();
        let ru_before = read_thread_rusage();
        let start = std::time::Instant::now();
        let trim_result = unsafe { libc::malloc_trim(0) };
        let duration_us = start.elapsed().as_micros() as i64;
        let rss_after_kb = read_statm_rss_kb();
        let ru_after = read_thread_rusage();

        let report = MallocTrimReport {
            supported: true,
            trim_result,
            rss_before_kb,
            rss_after_kb,
            duration_us,
            minflt_delta: ru_after
                .as_ref()
                .zip(ru_before.as_ref())
                .map(|(after, before)| after.ru_minflt - before.ru_minflt)
                .unwrap_or(-1),
            majflt_delta: ru_after
                .as_ref()
                .zip(ru_before.as_ref())
                .map(|(after, before)| after.ru_majflt - before.ru_majflt)
                .unwrap_or(-1),
        };

        logger.debug(&format!(
            "malloc_trim tag={tag} result={} rss_before={}kB rss_after={}kB delta={}kB duration_us={}",
            report.trim_result,
            report.rss_before_kb,
            report.rss_after_kb,
            report.delta_kb(),
            report.duration_us
        ));
        report
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        logger.debug(&format!(
            "malloc_trim not supported on this platform (tag={tag})"
        ));
        MallocTrimReport::default()
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn read_statm_rss_kb() -> i64 {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return -1;
    };
    let mut fields = statm.split_whitespace();
    let _size = fields.next();
    let Some(resident) = fields.next() else {
        return -1;
    };
    let Ok(resident_pages) = resident.parse::<i64>() else {
        return -1;
    };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return -1;
    }
    resident_pages * page_size / 1024
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn read_thread_rusage() -> Option<libc::rusage> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) };
    if result == 0 {
        Some(unsafe { usage.assume_init() })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{MallocTrimLogger, NullMallocTrimLogger, malloc_trim};
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct RecordingLogger {
        entries: Mutex<Vec<String>>,
    }

    impl MallocTrimLogger for RecordingLogger {
        fn debug(&self, message: &str) {
            self.entries
                .lock()
                .expect("logger mutex poisoned")
                .push(message.to_owned());
        }
    }

    #[test]
    fn malloc_trim_reports_support_state_and_logs() {
        let logger = RecordingLogger::default();
        let report = malloc_trim("test", &logger);
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        assert!(report.supported);
        #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
        assert!(!report.supported);
        assert!(
            !logger
                .entries
                .lock()
                .expect("logger mutex poisoned")
                .is_empty()
        );

        let null_logger = NullMallocTrimLogger;
        let _ = malloc_trim("null", &null_logger);
    }
}
