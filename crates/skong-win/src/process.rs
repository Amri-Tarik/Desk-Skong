//! Own-process resource usage (CPU % since the previous sample, working set).
use std::time::Instant;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

/// Samples CPU time deltas; the first sample covers the time since creation.
pub struct UsageSampler {
    last: (Instant, f64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    /// Of one core, since the previous sample.
    pub cpu_percent: f32,
    pub working_set_bytes: u64,
}

impl Default for UsageSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageSampler {
    pub fn new() -> Self {
        Self { last: (Instant::now(), cpu_seconds()) }
    }

    pub fn sample(&mut self) -> Usage {
        let (now, cpu) = (Instant::now(), cpu_seconds());
        let wall = now.duration_since(self.last.0).as_secs_f64();
        let cpu_percent = if wall > 0.0 { (100.0 * (cpu - self.last.1) / wall) as f32 } else { 0.0 };
        self.last = (now, cpu);
        Usage { cpu_percent: cpu_percent.max(0.0), working_set_bytes: working_set() }
    }
}

/// User + kernel CPU seconds of this process.
pub fn cpu_seconds() -> f64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: querying our own process; out-pointers reference locals.
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.is_err() {
        return 0.0;
    }
    let secs = |f: FILETIME| ((u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)) as f64 * 1e-7;
    secs(k) + secs(u)
}

fn working_set() -> u64 {
    let mut m = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    // SAFETY: our own process; `m` is properly sized.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) }.as_bool() { m.WorkingSetSize as u64 } else { 0 }
}
