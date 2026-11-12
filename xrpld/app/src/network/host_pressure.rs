//! Host pressure for slow-turn logs: Linux PSI (`/proc/pressure/*`) and the
//! process's major page faults.
//!
//! IMPORTANT: on Nov 11–12 every consensus stall fell in a 10-minute window
//! with 500–950 major faults/s (sar), against under 10/s otherwise. A slow
//! turn's log line now carries the faults taken during that turn and the PSI
//! stall time, so a pause caused by paging or I/O is visible per event.
//!
//! PSI line format (`some` = at least one task stalled, `full` = all stalled):
//! `some avg10=1.23 avg60=0.50 avg300=0.10 total=123456789` (total in µs).

use std::fmt::Write as _;

/// Cumulative counters at one instant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PressureSample {
    pub major_faults: u64,
    /// `some` stall totals (µs) for cpu, memory, io; 0 when PSI is missing.
    pub cpu_some_us: u64,
    pub memory_some_us: u64,
    pub memory_full_us: u64,
    pub io_some_us: u64,
    pub io_full_us: u64,
}

/// Major faults of the whole process so far (getrusage, one syscall).
pub fn major_faults() -> u64 {
    #[cfg(unix)]
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
            return usage.ru_majflt as u64;
        }
    }
    0
}

/// (`some` total, `full` total) in µs from one `/proc/pressure/*` file.
pub fn parse_psi(text: &str) -> (u64, u64) {
    let total = |kind: &str| {
        text.lines()
            .find(|line| line.starts_with(kind))
            .and_then(|line| line.split_whitespace().find_map(|f| f.strip_prefix("total=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    (total("some "), total("full "))
}

fn read_psi(resource: &str) -> (u64, u64) {
    std::fs::read_to_string(format!("/proc/pressure/{resource}"))
        .map(|text| parse_psi(&text))
        .unwrap_or((0, 0))
}

/// Read every counter. A few small /proc reads; call at turn boundaries.
pub fn sample() -> PressureSample {
    let (cpu_some_us, _) = read_psi("cpu");
    let (memory_some_us, memory_full_us) = read_psi("memory");
    let (io_some_us, io_full_us) = read_psi("io");
    PressureSample {
        major_faults: major_faults(),
        cpu_some_us,
        memory_some_us,
        memory_full_us,
        io_some_us,
        io_full_us,
    }
}

/// What changed between two samples, as one log field:
/// `majflt=… mem_some_ms=… mem_full_ms=… io_some_ms=… io_full_ms=… cpu_some_ms=…`.
pub fn describe_delta(before: &PressureSample, after: &PressureSample) -> String {
    let ms = |a: u64, b: u64| b.saturating_sub(a) / 1000;
    let mut out = String::new();
    let _ = write!(
        out,
        "majflt={} mem_some_ms={} mem_full_ms={} io_some_ms={} io_full_ms={} cpu_some_ms={}",
        after.major_faults.saturating_sub(before.major_faults),
        ms(before.memory_some_us, after.memory_some_us),
        ms(before.memory_full_us, after.memory_full_us),
        ms(before.io_some_us, after.io_some_us),
        ms(before.io_full_us, after.io_full_us),
        ms(before.cpu_some_us, after.cpu_some_us),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psi_totals_are_parsed_for_some_and_full() {
        let text = "some avg10=1.50 avg60=0.40 avg300=0.10 total=9876543\n\
                    full avg10=0.20 avg60=0.05 avg300=0.01 total=1234567\n";
        assert_eq!(parse_psi(text), (9_876_543, 1_234_567));
        // cpu has only a `some` line on older kernels.
        assert_eq!(parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=42\n"), (42, 0));
        assert_eq!(parse_psi(""), (0, 0));
    }

    #[test]
    fn delta_reports_faults_and_stall_milliseconds() {
        let before = PressureSample {
            major_faults: 100,
            cpu_some_us: 1_000,
            memory_some_us: 10_000,
            memory_full_us: 5_000,
            io_some_us: 20_000,
            io_full_us: 0,
        };
        let after = PressureSample {
            major_faults: 4_200,
            cpu_some_us: 3_000,
            memory_some_us: 5_810_000,
            memory_full_us: 2_005_000,
            io_some_us: 1_220_000,
            io_full_us: 300_000,
        };
        assert_eq!(
            describe_delta(&before, &after),
            "majflt=4100 mem_some_ms=5800 mem_full_ms=2000 io_some_ms=1200 io_full_ms=300 cpu_some_ms=2"
        );
    }

    #[test]
    fn sample_reads_this_process_fault_counter() {
        let s = sample();
        // getrusage always works on unix; the PSI files may be absent (macOS).
        let _ = s.major_faults;
        assert!(describe_delta(&s, &s).starts_with("majflt=0 "));
    }
}
