//! Filesystem support shared by public and private real-server fixtures and their launcher binaries.
//! The checkout-specific scratch root is defined once, independently of per-test HOME and TMPDIR overrides.

#[path = "../tests/common/test_paths.rs"]
pub mod test_paths;

/// Pinned Vibe runtime selection shared by every real-server rig.
pub mod runtime;
