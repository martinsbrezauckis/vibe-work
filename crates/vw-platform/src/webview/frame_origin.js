// Capture the security origin before page scripts run. WebView2 installs this on every document navigation.
// The native frame evaluator reads this immutable primitive through a null-prototype result envelope.
Object.defineProperty(window, "__vwNativeFrameOrigin", { value: window.origin });
