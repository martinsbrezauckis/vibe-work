//! Browser diagnostics preserve ordinary console levels and resource failures within existing bounds.
//! Tests the native drain parser independently of page text and host engine availability.
use serde_json::json;

#[test]
fn every_console_level_and_resource_failure_survives_the_drain() {
    let levels = [
        "log",
        "info",
        "debug",
        "warn",
        "error",
        "exception",
        "rejection",
        "resource",
    ];
    let data = json!(levels.map(
        |level| json!({"level":level,"text":"example","url":"https://example.test/","atMs":1})
    ));
    let entries = vw_preview::parse_drain(&data.to_string());
    assert_eq!(entries.len(), levels.len());
    for (entry, level) in entries.iter().zip(levels) {
        assert_eq!(serde_json::to_value(entry).unwrap()["level"], level);
    }
}

#[test]
fn capture_keeps_console_calls_exceptions_rejections_and_resource_urls() {
    let init = vw_preview::init_script("__console_fixture");
    let drain = vw_preview::drain_script("__console_fixture");
    let source = format!(
        r"
        const vm = require('node:vm');
        const listeners = new Map();
        const context = {{window: null, location: {{href:'https://example.test/'}}, console: {{}}}};
        context.window = context;
        context.addEventListener = (name, callback) => listeners.set(name, callback);
        for (const level of ['log','info','debug','warn','error']) context.console[level] = () => {{}};
        vm.createContext(context);
        vm.runInContext({init}, context);
        for (const level of ['log','info','debug','warn','error']) context.console[level]('message');
        listeners.get('error')({{message:'failed script',target:context}});
        listeners.get('error')({{target:{{src:'https://example.test/missing.png'}}}});
        listeners.get('unhandledrejection')({{reason:'rejected promise'}});
        process.stdout.write(vm.runInContext({drain}, context));
    ",
        init = json!(init),
        drain = json!(drain)
    );
    let output = std::process::Command::new("node")
        .args(["-e", &source])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entries = vw_preview::parse_drain(std::str::from_utf8(&output.stdout).unwrap());
    assert_eq!(entries.len(), 8);
    assert_eq!(
        entries.iter().filter(|e| e.level.is_page_error()).count(),
        3
    );
    assert!(entries.iter().any(|e| e.text.contains("missing.png")));
}
