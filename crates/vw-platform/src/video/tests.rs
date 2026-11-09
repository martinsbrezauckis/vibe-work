//! Owned-corpus decoder regressions. Native tests require `VW_VIDEO_CORPUS` and never claim sandbox proof.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Bounded coordinates in the owned 640px fixture barcode.
use super::*;
use std::path::Path;
fn open(name: &str) -> Decoder {
    let root =
        std::env::var_os("VW_VIDEO_CORPUS").expect("VW_VIDEO_CORPUS must name the verified corpus");
    Decoder::open(&Path::new(&root).join(name), 1280).unwrap()
}
fn barcode(frame: &Frame, y: f64) -> u32 {
    let bits: Vec<_> = (0..28)
        .map(|i| {
            let x = ((f64::from(i) + 2.5) * f64::from(frame.width) / 32.0) as usize;
            let y = (y * f64::from(frame.height)) as usize;
            let at = (y * frame.width as usize + x) * 4;
            frame.pixels[at..at + 3]
                .iter()
                .map(|b| u32::from(*b))
                .sum::<u32>()
                > 384
        })
        .collect();
    assert_eq!(&bits[..2], &[true, false]);
    assert_eq!(&bits[26..], &[false, true]);
    (0..24).fold(0, |v, i| v | (u32::from(bits[i + 2]) << i))
}
#[test]
#[ignore = "requires the owned video corpus on a native host"]
fn vfr_covering_frame_preserves_original_pts() {
    let mut d = open("h264-vfr.mp4");
    for (ms, index, pts) in [(999, 19, 920), (2501, 50, 2480)] {
        let f = d.frame(ms * 1_000_000, 640).unwrap();
        assert_eq!(barcode(&f, 0.11), index);
        assert_eq!(barcode(&f, 0.21), pts);
        assert_eq!(f.pts_ns, u64::from(pts) * 1_000_000);
    }
}
#[test]
#[ignore = "requires the owned video corpus on a native host"]
fn bframes_and_eof_never_reuse_stale_frames() {
    for file in [
        "h264-main-bframes.mp4",
        "hevc-main.mp4",
        "h264-fragmented.mp4",
    ] {
        let root = std::env::var_os("VW_VIDEO_CORPUS").expect("verified corpus");
        let mut d = match Decoder::open(&Path::new(&root).join(file), 1280) {
            Ok(decoder) => decoder,
            Err(reason)
                if !cfg!(target_os = "linux")
                    && file == "hevc-main.mp4"
                    && reason.starts_with("unsupported_codec:") =>
            {
                eprintln!(
                    "NOT RUN: HEVC covering frames; this computer cannot decode it: {reason}"
                );
                continue;
            }
            Err(reason) => panic!("{file}: {reason}"),
        };
        let duration = d.metadata().unwrap().duration_ns;
        assert!(d.frame(duration, 640).is_err());
        for ms in [41, 999, 2501, 0, 3999, 41] {
            let f = d.frame(ms * 1_000_000, 640).unwrap();
            assert_eq!(barcode(&f, 0.11), (ms / 40) as u32);
            assert_eq!(f.pts_ns, (ms / 40) * 40 * 1_000_000);
        }
    }
}
#[test]
#[ignore = "requires the owned video corpus on a native host"]
fn rotation_and_pixel_aspect_are_display_geometry() {
    let mut d = open("h264-rot90.mp4");
    let f = d.frame(999_000_000, 640).unwrap();
    assert_eq!((f.width, f.height), (360, 640));
    assert_eq!(d.metadata().unwrap().rotation, 90);
    let mut d = open("h264-par.mp4");
    let f = d.frame(999_000_000, 1280).unwrap();
    assert_eq!((f.width, f.height), (853, 360));
}
#[test]
#[ignore = "requires the owned video corpus on a native host"]
fn malformed_and_truncated_never_produce_ready_frames() {
    for file in ["malformed-mp4.mp4", "truncated-mp4.mp4"] {
        if let Ok(mut d) = Decoder::open(
            &Path::new(&std::env::var_os("VW_VIDEO_CORPUS").unwrap()).join(file),
            1280,
        ) {
            assert!(d.frame(3_999_000_000, 640).is_err(), "{file}");
        }
    }
}

#[test]
#[ignore = "requires the owned video corpus on a native host"]
fn every_vfr_transition_selects_the_covering_barcode() {
    let mut d = open("h264-vfr.mp4");
    // The owned fixture repeats these four presentation intervals, totaling four seconds.
    let mut pts = vec![0u64];
    for i in 0..79 {
        pts.push(pts[i] + [20, 60, 40, 80][i % 4]);
    }
    for (index, &at) in pts.iter().enumerate().skip(1) {
        for time in [at - 1, at, at + 1] {
            let expected = if time < at { index - 1 } else { index };
            let f = d.frame(time * 1_000_000, 640).unwrap();
            assert_eq!(barcode(&f, 0.11), expected as u32, "time {time}");
            assert_eq!(barcode(&f, 0.21), pts[expected] as u32, "time {time}");
            assert_eq!(f.pts_ns, pts[expected] * 1_000_000);
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn resource_caps_are_enforced_in_a_child() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "video::tests::resource_caps_child",
            "--ignored",
            "--nocapture",
        ])
        .env("VW_VIDEO_LIMIT_TEST_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[cfg(target_os = "linux")]
#[ignore = "installs irreversible process limits; only the parent test invokes it"]
fn resource_caps_child() {
    if std::env::var_os("VW_VIDEO_LIMIT_TEST_CHILD").is_none() {
        return;
    }
    super::limits::restrict().unwrap();
    assert_eq!(
        rustix::process::getrlimit(rustix::process::Resource::As).maximum,
        Some(1024 * 1024 * 1024)
    );
    assert_eq!(
        rustix::process::getrlimit(rustix::process::Resource::Fsize).maximum,
        Some(0)
    );
    assert!(std::net::TcpListener::bind("127.0.0.1:0").is_err());
    assert!(std::process::Command::new("/bin/true").status().is_err());
    // SAFETY: no address is dereferenced; an unexpectedly successful mapping is immediately released.
    unsafe {
        let mapping = libc::mmap(
            std::ptr::null_mut(),
            2 * 1024 * 1024 * 1024,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        if mapping != libc::MAP_FAILED {
            libc::munmap(mapping, 2 * 1024 * 1024 * 1024);
            panic!("memory cap was not enforced");
        }
    }
}
