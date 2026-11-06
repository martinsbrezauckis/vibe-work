//! Windows Job budgets, shared by launch and verification. Pure policy tests run on every host.
//! The parent deadline bounds elapsed time; Job user time counts all decoder threads.
// Media Foundation's software decoder needs about 830 MB for a 4K H.264 frame.
pub(super) const MEMORY: usize = 1024 * 1024 * 1024;

pub(super) fn cpu_time(parallelism: usize) -> i64 {
    // A 4K scene scan needs over 40 s of aggregate user time, even inside 35 s elapsed.
    // Permit at most what the available CPUs can spend before the parent's unchanged deadline.
    let cores = i64::try_from(parallelism).unwrap_or(1).max(1);
    let ticks = i64::try_from(super::DEADLINE.as_nanos() / 100).unwrap_or(i64::MAX);
    ticks.saturating_mul(cores)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_uhd_decoder_has_one_gib_of_job_memory() {
        assert_eq!(super::MEMORY, 1024 * 1024 * 1024);
    }

    #[test]
    fn aggregate_cpu_time_fits_the_existing_wall_deadline() {
        assert_eq!(super::super::DEADLINE.as_secs(), 35);
        for (cores, seconds) in [(1, 35), (2, 70), (64, 2240)] {
            assert_eq!(super::cpu_time(cores), seconds * 10_000_000);
        }
    }
}
