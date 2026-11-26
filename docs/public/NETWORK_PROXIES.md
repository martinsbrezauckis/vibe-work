# Network proxies

The optional proxy support lets the built-in browser use HTTP, HTTPS and SOCKS proxies, including those selected by the computer's automatic proxy settings. It is included in release builds.

For public destinations, the browser uses saved Settings > Network proxy values first. Without a matching saved proxy, it asks the computer's proxy settings. A company certificate saved in Settings is also used to verify an HTTPS proxy. Restart the app after changing saved settings.

On Windows and macOS, automatic settings also apply to Vibe Work's model requests, key checks, OCR and HTTP speech requests, and to Vibe's HTTP clients through its proxy environment. A configured proxy keeps its existing behavior; an unconfigured request type uses the system's selection. Public capture media and live dictation follow the selected proxy route. Permitted local capture media stays direct. The core and Vibe keep their existing environment precedence. Their connection to the local model proxy stays direct.

When the computer's proxy settings change while Vibe Work runs, Vibe Work's own requests use the new settings from their next request; connections the built-in browser or the agent's tools already have open switch when they next connect, and restarting Vibe Work applies the change everywhere.

Update checks use the verified Network choice. If a saved record still needs a vault unlock or the automatic route is not ready, Check for updates reports that the network route is not ready and sends no update request. A verified manual HTTPS proxy can be used as soon as its settings are loaded.

The connection test says whether it connected directly or through a proxy. Test connection names an unreachable proxy. A failed explicit PAC lookup is a failed connection, never permission to try directly. Windows uses the configured PAC URL, or WPAD when automatic detection is enabled in the system settings. If WPAD is unavailable, Windows keeps its existing manual or default proxy fallback.

HTTPS proxy addresses begin with `https://`: the connection to the proxy is encrypted and its certificate and name are verified. A proxy listed under a system setting named “HTTPS” may instead be an ordinary HTTP proxy used for HTTPS destinations; on macOS, that label means HTTP CONNECT.

SOCKS5 supports username/password sign-in. SOCKS4 and SOCKS4a support a user ID, IPv4 destinations and no password. Use SOCKS5 for IPv6 or password sign-in. Proxy credentials belong in Settings or the computer's proxy settings, never in chat.

The browser resolves names locally and sends the resulting address to the proxy, including with `socks5h://` or `socks4a://`. Permitted local and private destinations keep their checked direct route. A destination that only the proxy can resolve may fail to open. Per-chat permissions still apply.

Automatic core and Vibe routes also resolve names on this computer before contacting the selected proxy. These service requests retain access to local and private services; they do not grant the browser new permissions. Manual core proxy routes keep their existing name-resolution behavior.

Automatic proxy lists are tried in order. A direct connection still uses the resolved destination address. PAC execution belongs to the operating system or libproxy; the browser does not evaluate scripts itself.

On Windows, a PAC script that returns an empty answer is read by Windows as DIRECT, and Vibe Work follows that; an answer Windows cannot read is refused.

On Linux, automatic and desktop proxy discovery requires libproxy 0.5.5 or newer with its desktop configuration and PAC backends. If the library is missing or automatic lookup fails, enter a manual proxy for the required request types in Settings > Network or install the distribution's libproxy package and backends. The browser reports the lookup failure instead of bypassing it. Manual proxy environment variables also work. PAC URLs stay in the desktop's automatic proxy setting.

On macOS, automatic proxy lists can contain several choices. A failed PAC proxy does not itself permit a direct retry. An explicit DIRECT result still permits a direct connection. PAC proxies for HTTPS addresses are used as ordinary HTTP proxies, as macOS itself does. Use an explicit `https://` address in Settings for a proxy that needs TLS. The operating system's ordinary HTTP and SOCKS PAC results are supported.

On Windows, native automatic proxy results support SOCKS4. If an automatic setup requires SOCKS5, enter its explicit `socks5://` or `socks5h://` address in Settings.
