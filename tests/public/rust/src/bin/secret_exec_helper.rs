//! Tiny executable fixtures for secret-run tests. Uses only std; arguments select echo, environment
//! output or executable-path inspection. Accepts arbitrary filenames when copied or run by descriptor.

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("echo") => println!("{}", args[1..].join(" ")),
        Some("printenv") => println!("{}", std::env::var(&args[1]).unwrap()),
        #[cfg(target_os = "linux")]
        Some("exe") => println!(
            "{}",
            std::fs::read_link("/proc/self/exe").unwrap().display()
        ),
        _ => panic!("expected echo, printenv or exe"),
    }
}
