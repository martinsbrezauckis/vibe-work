//! Per-thread background scheduling for file work. Depends on native scheduling APIs;
//! the core scanner calls it once inside its dedicated worker, never on the UI or runtime threads.

/// Lower only the calling thread's scheduling priority.
///
/// # Errors
/// The OS refused to change this thread's priority.
pub fn lower_thread_priority() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        rustix::process::nice(10).map_err(std::io::Error::from)?;
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: the pseudo-handle names the calling thread and is not closed or retained.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) }
            .map_err(std::io::Error::other)?;
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
        }
        // SAFETY: the API changes the calling thread only; background QoS and relative priority zero are valid.
        let error = unsafe { pthread_set_qos_class_self_np(0x09, 0) };
        if error != 0 {
            return Err(std::io::Error::from_raw_os_error(error));
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn lowers_worker_priority_without_lowering_the_foreground_thread() {
        let foreground = rustix::process::getpriority_process(None).unwrap();
        let worker = std::thread::spawn(move || {
            lower_thread_priority().unwrap();
            assert_eq!(
                rustix::process::getpriority_process(None).unwrap(),
                (foreground + 10).min(19)
            );
        });
        worker.join().unwrap();
        assert_eq!(
            rustix::process::getpriority_process(None).unwrap(),
            foreground
        );
    }
}
