"""Keep system-trust TLS on the OS roots in both Vibe 3.1.4 HTTP stacks.

Prepended to the launcher before app-server imports. Vibe 3.1.4 adds certifi roots to
its system context; Vibe Work's existing setting promises OS roots alone. The
person's explicit false choice still uses Vibe's original builder, and custom
SSL_CERT_FILE/SSL_CERT_DIR roots remain additive in either mode.
"""


def install_system_trust_guard():
    require_guarded_vibe("OS trust-store guard")  # noqa: F821 - launch.rs prepends shared versions
    import functools
    import os
    import ssl
    import sys

    from vibe.utils import http as vibe_http
    from mistralai_vibe_local_harness.vibe import _ssl as harness_ssl

    if getattr(vibe_http.build_ssl_context, "_vw_os_only", False):
        return

    old_http = vibe_http.build_ssl_context
    old_harness = harness_ssl.build_ssl_context
    old_http.cache_clear()
    old_harness.cache_clear()

    @functools.lru_cache(maxsize=1)
    def http_context():
        if not vibe_http.system_trust_store_enabled():
            return old_http()
        context = vibe_http.truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        cert_file = os.getenv("SSL_CERT_FILE")
        cert_dir = os.getenv("SSL_CERT_DIR")
        if cert_file or cert_dir:
            try:
                context.load_verify_locations(cafile=cert_file, capath=cert_dir)
            except (OSError, ssl.SSLError):
                from vibe.observability.logging import logger
                logger.warning(
                    "Failed to load custom SSL certificates: SSL_CERT_FILE=%s SSL_CERT_DIR=%s",
                    cert_file, cert_dir,
                )
        return context

    @functools.cache
    def harness_context(*, use_system_trust_store=False):
        if not use_system_trust_store:
            return old_harness(use_system_trust_store=False)
        context = harness_ssl.truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        return harness_ssl._add_custom_roots(context)

    http_context._vw_os_only = True
    harness_context._vw_os_only = True
    vibe_http.build_ssl_context = http_context
    harness_ssl.build_ssl_context = harness_context
    # Importing the harness _ssl submodule also executes its parent package,
    # whose transport and adapters bind the old function with `from ... import`.
    # Replace every already-bound alias by identity before app-server startup.
    for module in tuple(sys.modules.values()):
        if getattr(module, "build_ssl_context", None) is old_harness:
            module.build_ssl_context = harness_context
        elif getattr(module, "build_ssl_context", None) is old_http:
            module.build_ssl_context = http_context
