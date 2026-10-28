//! Synthetic PE resources and distlib overlays, plus native Windows planted-interpreter execution.
//! Exercises the resolver without Python, Vibe, or a real user environment.
use super::launcher::interpreter;
use std::path::PathBuf;

fn put16(bytes: &mut [u8], at: usize, n: u16) {
    bytes[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, n: u32) {
    bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn pe() -> Vec<u8> {
    let mut bytes = vec![0; 1536];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 0x86, 1);
    put16(&mut bytes, 0x94, 240);
    put16(&mut bytes, 0x98, 0x20b);
    put32(&mut bytes, 0x98 + 112 + 16, 0x1000);
    put32(&mut bytes, 0x188 + 12, 0x1000);
    put32(&mut bytes, 0x188 + 16, 1024);
    put32(&mut bytes, 0x188 + 20, 512);
    bytes
}
fn uv(path: &str) -> Vec<u8> {
    let mut bytes = pe();
    let r = &mut bytes[512..];
    put16(r, 14, 1);
    put32(r, 16, 10);
    put32(r, 20, 0x8000_0020);
    put16(r, 32 + 12, 3);
    let mut name_at = 224;
    let mut data_at = 512;
    for (i, (name, data)) in [
        ("UV_TRAMPOLINE_KIND", &[1][..]),
        ("UV_PYTHON_PATH", path.as_bytes()),
        ("UV_SCRIPT_DATA", &b"PK\x03\x04payload"[..]),
    ]
    .into_iter()
    .enumerate()
    {
        put32(r, 48 + i * 8, 0x8000_0000 | u32::try_from(name_at).unwrap());
        put32(
            r,
            52 + i * 8,
            0x8000_0000 | u32::try_from(80 + i * 24).unwrap(),
        );
        put16(r, 80 + i * 24 + 14, 1);
        put32(r, 80 + i * 24 + 20, u32::try_from(160 + i * 16).unwrap());
        put32(r, 160 + i * 16, 0x1000 + u32::try_from(data_at).unwrap());
        put32(r, 164 + i * 16, u32::try_from(data.len()).unwrap());
        put16(r, name_at, u16::try_from(name.len()).unwrap());
        for (n, ch) in name.encode_utf16().enumerate() {
            put16(r, name_at + 2 + n * 2, ch);
        }
        name_at += 2 + name.len() * 2;
        r[data_at..data_at + data.len()].copy_from_slice(data);
        data_at += data.len();
    }
    bytes
}
fn distlib(path: &str) -> Vec<u8> {
    let mut bytes = pe();
    bytes.extend_from_slice(format!("#!\"{path}\"\r\n").as_bytes());
    bytes.extend_from_slice(b"PK\x03\x04local");
    bytes.extend_from_slice(b"PK\x01\x02central");
    let mut end = [0u8; 22];
    end[..4].copy_from_slice(b"PK\x05\x06");
    put32(&mut end, 12, 11);
    put32(&mut end, 16, 9);
    bytes.extend_from_slice(&end);
    bytes
}

#[test]
fn unrelated_binary_strings_are_not_an_interpreter_association() {
    assert_eq!(interpreter(b"MZ\0C:\\planted\\python.exe\0"), None);
    let mut bytes = pe();
    bytes.extend_from_slice(b"noise#!C:\\planted\\python.exe\nPK\x03\x04");
    assert_eq!(interpreter(&bytes), None);
}
#[test]
fn uv_resource_and_distlib_overlay_resolve_the_named_interpreter() {
    let path = "C:\\Users\\Mārtiņš\\vibe env\\Scripts\\python.exe";
    assert_eq!(interpreter(&uv(path)), Some(PathBuf::from(path)));
    assert_eq!(interpreter(&distlib(path)), Some(PathBuf::from(path)));
    let mut bytes = uv(path);
    bytes.extend_from_slice(b"C:\\decoy\\python.exe\0");
    assert_eq!(interpreter(&bytes), Some(PathBuf::from(path)));
}
#[test]
fn malformed_resources_and_overlays_fail_closed() {
    for mut bytes in [uv("Scripts\\python.exe"), distlib("C:\\env\\python.exe")] {
        bytes.pop();
        assert_eq!(interpreter(&bytes), None);
    }
    let bytes = uv("C:\\env\\Scripts\\python.exe");
    for length in 0..bytes.len() {
        assert_eq!(interpreter(&bytes[..length]), None);
    }
    let mut bytes = bytes;
    put32(&mut bytes, 512 + 20, 0xffff_ffff);
    assert_eq!(interpreter(&bytes), None);
}

#[cfg(windows)]
#[test]
fn planted_sibling_recorder_never_runs() {
    let root = std::env::temp_dir().join(format!("vw-python-association-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("launchers")).unwrap();
    let scripts = root.join("env/Scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::create_dir_all(root.join("env/Lib/site-packages/vibe")).unwrap();
    std::fs::write(root.join("env/pyvenv.cfg"), "home = C:\\Python\n").unwrap();
    let python = scripts.join("python.exe");
    let sibling = root.join("launchers/python.exe");
    for (binary, marker) in [(&python, "trusted"), (&sibling, "planted")] {
        let source = root.join(format!("{marker}.rs"));
        std::fs::write(
            &source,
            format!(
                "fn main() {{ std::fs::write({:?}, b\"ran\").unwrap(); }}",
                root.join(marker)
            ),
        )
        .unwrap();
        assert!(
            std::process::Command::new("rustc")
                .arg(source)
                .arg("-o")
                .arg(binary)
                .status()
                .unwrap()
                .success()
        );
    }
    let server = root.join("launchers/vibe-app-server.exe");
    for bytes in [
        uv(python.to_str().unwrap()),
        distlib(python.to_str().unwrap()),
    ] {
        std::fs::write(&server, bytes).unwrap();
        let selected = super::server_python(&server).unwrap();
        assert!(
            std::process::Command::new(selected)
                .status()
                .unwrap()
                .success()
        );
        assert!(root.join("trusted").exists());
        assert!(!root.join("planted").exists());
    }
    std::fs::write(&server, format!("MZnoise{}", python.display())).unwrap();
    assert_eq!(super::server_python(&server), None);
    std::fs::write(&server, uv(sibling.to_str().unwrap())).unwrap();
    assert_eq!(super::server_python(&server), None);
    let script = scripts.join("vibe-app-server-script.py");
    std::fs::write(&script, b"#!python\n").unwrap();
    assert!(super::server_python(&script).is_some());
    std::fs::remove_dir_all(root).unwrap();
}
