# Network proxies

The optional proxy support lets the built-in browser use HTTP, HTTPS and SOCKS proxies, including those selected by the computer's automatic proxy settings. It is not enabled in the default build yet.

The browser uses saved Settings > Network proxy values first. Without a matching saved proxy, it asks the computer's proxy settings. A company certificate saved in Settings is also used to verify an HTTPS proxy. Restart the app after changing saved settings.

HTTPS proxy addresses begin with `https://`: the connection to the proxy is encrypted and its certificate and name are verified. A proxy listed under a system setting named “HTTPS” may instead be an ordinary HTTP proxy used for HTTPS destinations; on macOS, that label means HTTP CONNECT.

SOCKS5 supports username/password sign-in. SOCKS4 and SOCKS4a support a user ID, IPv4 destinations and no password. Use SOCKS5 for IPv6 or password sign-in. Proxy credentials belong in Settings or the computer's proxy settings, never in chat.

The browser checks the destination before using a proxy. It resolves names itself and gives the proxy only an address that passed those checks, including with `socks5h://` or `socks4a://`. A network where only the proxy can resolve a destination is refused: letting the proxy resolve it could reach an address the browser has not checked. Local pages and per-chat permissions keep their existing rules.

Automatic proxy lists are tried in order. A direct connection still uses the checked destination address. PAC execution belongs to the operating system or libproxy; the browser does not evaluate scripts itself.

On Linux, automatic and desktop proxy discovery requires libproxy 0.5.5 or newer with its desktop configuration and PAC backends. If the library is missing or automatic lookup fails, enter a manual proxy for the required request types in Settings > Network or install the distribution's libproxy package and backends. The browser reports the lookup failure instead of bypassing it. Manual proxy environment variables also work. PAC URLs stay in the desktop's automatic proxy setting.

On macOS, PAC results that identify an ambiguous HTTPS proxy are refused. Use an explicit `https://` address in Settings for a proxy that needs TLS. The operating system's ordinary HTTP and SOCKS PAC results are supported.

On Windows, native automatic proxy results support SOCKS4. If an automatic setup requires SOCKS5, enter its explicit `socks5://` or `socks5h://` address in Settings.
