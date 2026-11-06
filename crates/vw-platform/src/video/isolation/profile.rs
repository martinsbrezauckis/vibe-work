//! Seatbelt policy construction. Paths are literals, never executable profile syntax.
use std::path::Path;

// Rules beyond the deny-by-default base, each tied to the native denial it fixes:
// - `iokit-open IOSurfaceRootUserClient`: without it Seatbelt reports `deny(1) iokit-open-user-client
//   IOSurfaceRootUserClient`, VideoToolbox's hardware decoder (VTDecoderXPCService) cannot share the decoded
//   IOSurface back (`PixelBufferSharing err=-15595`, "decoded 0 dropped 1") and every frame fails to decode.
//   IOSurface is the shared-memory handle for decoded pixel buffers; it grants no file, network or process access.
// Other reported denials (preferences, GPU device, window server, dirhelper, TCC, disk space) are not needed for
// extraction and stay denied.
pub(super) fn profile(input: &Path) -> Result<String, String> {
    let path = input
        .to_str()
        .filter(|s| s.starts_with('/') && !s.chars().any(char::is_control))
        .ok_or("backend_unavailable: sandbox input path")?;
    let literal = path.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!(
        r#"(version 1)
(deny default)
(allow signal (target self))
(allow sysctl-read)
(allow file-read-metadata)
(allow file-read* (subpath "/System") (subpath "/usr/lib")
    (literal "/dev/urandom") (literal "/dev/random") (literal "{literal}"))
(allow mach-lookup
    (global-name "com.apple.cfprefsd.daemon")
    (global-name "com.apple.system.logger")
    (global-name "com.apple.coremedia.videodecoder"))
(allow iokit-open (iokit-user-client-class "IOSurfaceRootUserClient"))
"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_is_read_only_and_cannot_inject_a_rule() {
        let text = profile(Path::new("/private/tmp/a\" (allow network*) ;")).unwrap();
        assert!(text.contains("(deny default)"));
        assert!(text.contains(r#"(literal "/private/tmp/a\" (allow network*) ;")"#));
        assert!(!text.lines().any(|line| line.starts_with("(allow network")));
        assert!(!text.contains("(allow file-write"));
        assert!(!text.contains("(allow process"));
        assert!(!text.contains("(allow iokit-open)"));
        assert!(text.contains(r#"(iokit-user-client-class "IOSurfaceRootUserClient")"#));
        for path in ["relative", "/tmp/a\n(allow default)", "/tmp/\0"] {
            assert!(profile(Path::new(path)).is_err());
        }
    }
}
