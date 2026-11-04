//! Linux helper resource and syscall limits, installed before any untrusted media parsing.
//! Bubblewrap supplies filesystem, PID and network namespaces; these caps also deny child process escapes.
/// Applies hard address-space, CPU, file and syscall limits to the current helper.
/// # Errors
/// An unsupported kernel or a denied limit installation; decoding must not start.
#[cfg(target_os = "linux")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Linux x86_64 syscall and errno constants are nonnegative 32-bit BPF values"
)]
pub fn restrict() -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    for (resource, cap) in [
        (Resource::As, 512 * 1024 * 1024),
        (Resource::Cpu, 35),
        (Resource::Fsize, 0),
        (Resource::Core, 0),
        (Resource::Nofile, 64),
    ] {
        setrlimit(
            resource,
            Rlimit {
                current: Some(cap),
                maximum: Some(cap),
            },
        )
        .map_err(|_| "backend_unavailable: helper resource limits")?;
    }
    #[cfg(target_arch = "x86_64")]
    {
        use libc::{sock_filter, sock_fprog};
        let instruction = |code, jt, jf, k| sock_filter { code, jt, jf, k };
        let mut filter = vec![
            instruction(0x20, 0, 0, 4),
            instruction(0x15, 1, 0, 0xc000_003e),
            instruction(0x06, 0, 0, 0x8000_0000),
            instruction(0x20, 0, 0, 0),
            // The x32 ABI uses the same audit architecture with a different syscall table.
            instruction(0x45, 0, 1, 0x4000_0000),
            instruction(0x06, 0, 0, 0x8000_0000),
        ];
        for syscall in [
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_fork,
            libc::SYS_vfork,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_mount,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_setsid,
            libc::SYS_setpgid,
            libc::SYS_bpf,
            libc::SYS_userfaultfd,
        ] {
            filter.push(instruction(0x15, 0, 1, syscall as u32));
            filter.push(instruction(0x06, 0, 0, 0x0005_0000 | libc::EPERM as u32));
        }
        filter.push(instruction(0x15, 0, 1, libc::SYS_clone3 as u32));
        filter.push(instruction(0x06, 0, 0, 0x0005_0000 | libc::ENOSYS as u32));
        // Only clone(CLONE_THREAD) is allowed; GStreamer threads cannot create another address space.
        filter.extend([
            instruction(0x15, 0, 4, libc::SYS_clone as u32),
            instruction(0x20, 0, 0, 16),
            instruction(0x45, 1, 0, libc::CLONE_THREAD as u32),
            instruction(0x06, 0, 0, 0x0005_0000 | libc::EPERM as u32),
            instruction(0x06, 0, 0, 0x7fff_0000),
            instruction(0x06, 0, 0, 0x7fff_0000),
        ]);
        let program = sock_fprog {
            len: u16::try_from(filter.len()).map_err(|_| "backend_unavailable: seccomp size")?,
            filter: filter.as_mut_ptr(),
        };
        // SAFETY: the kernel reads a valid BPF array for the duration of prctl, copying it before return.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
            || unsafe {
                libc::prctl(
                    libc::PR_SET_SECCOMP,
                    libc::SECCOMP_MODE_FILTER,
                    &raw const program,
                )
            } != 0
        {
            return Err("backend_unavailable: helper syscall sandbox".into());
        }
        Ok(())
    }
    #[cfg(not(target_arch = "x86_64"))]
    Err("backend_unavailable: helper syscall sandbox architecture".into())
}
