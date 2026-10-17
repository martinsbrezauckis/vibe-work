//! User-local IPC setup for the broker and shim, using interprocess and a private temp directory.
//! Unix directory permissions and Windows's explicit user SID DACL keep other users out.

use interprocess::local_socket::{Listener, ListenerOptions, Name, prelude::*};
use std::{io, path::Path};

#[cfg(unix)]
pub(crate) fn name(endpoint: &str) -> io::Result<Name<'_>> {
    endpoint.to_fs_name::<interprocess::local_socket::GenericFilePath>()
}

#[cfg(windows)]
pub(crate) fn name(endpoint: &str) -> io::Result<Name<'_>> {
    endpoint.to_ns_name::<interprocess::local_socket::GenericNamespaced>()
}

pub(crate) fn bind(directory: &Path) -> io::Result<(Listener, String)> {
    #[cfg(unix)]
    let endpoint = directory
        .join("broker.sock")
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "endpoint path must be UTF-8"))?
        .to_owned();
    #[cfg(windows)]
    let endpoint = format!(
        "vw-gh-{}",
        directory
            .file_name()
            .ok_or_else(|| io::Error::other("missing endpoint name"))?
            .to_string_lossy()
    );
    let options = ListenerOptions::new().name(name(&endpoint)?);
    #[cfg(windows)]
    let options = user_only(options)?;
    let listener = options.create_sync()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok((listener, endpoint))
}

#[cfg(windows)]
fn user_only(options: ListenerOptions<'_>) -> io::Result<ListenerOptions<'_>> {
    use interprocess::os::windows::{
        local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
    };
    let sid = windows_permissions::utilities::current_process_sid()?.to_string();
    let sddl = widestring::U16CString::from_str(format!("O:{sid}D:P(A;;GA;;;{sid})"))
        .map_err(io::Error::other)?;
    Ok(options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?))
}
