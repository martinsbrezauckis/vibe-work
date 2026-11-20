"""Close Vibe app-server telemetry, crash, experiment and tracing sinks.

The launcher calls this after version admission and before importing stdio.
"""


def install_privacy_guard():
    """Make the guarded runtime's privacy policy independent of config layers."""
    import os

    # The environment layer controls the settings shown to clients. Profile,
    # override and admin layers may outrank it, so the send sinks are closed too.
    os.environ.pop("SENTRY_DSN", None)
    os.environ.pop("VIBE_CRASH_REPORT_DSN", None)
    os.environ["VIBE_ENABLE_TELEMETRY"] = "false"
    os.environ["VIBE_ENABLE_OTEL"] = "false"
    os.environ["VIBE_EXPERIMENTS__ENABLE"] = "false"

    from vibe.core.telemetry.send import TelemetryClient
    from vibe.observability import sentry
    from vibe.core.experiments.client import RemoteEvalClient
    from vibe.core.experiments import session as experiments_session
    from vibe.core import tracing
    from vibe.app_server._service_resources import TelemetryResource

    TelemetryClient._is_enabled = lambda self: False

    # Both app-server CLI reports and the wheel's injected ACP DSN use _init.
    # Clearing only environment DSNs would leave the embedded one available.
    sentry._cli_dsn = None
    sentry._started = False

    def discard_cli_dsn():
        os.environ.pop("VIBE_CRASH_REPORT_DSN", None)
        sentry._cli_dsn = None

    sentry.claim_cli_crash_report_dsn = discard_cli_dsn
    sentry._init = lambda **_kwargs: False
    sentry.capture_sentry_exception = lambda *_args, **_kwargs: None

    async def no_remote_eval(self, attributes):
        return None

    RemoteEvalClient.evaluate = no_remote_eval

    async def no_plan_attributes(**_kwargs):
        return None, None

    async def no_experiments(**_kwargs):
        return False, None

    experiments_session.resolve_plan_attributes = no_plan_attributes
    experiments_session.initialize_experiments = no_experiments
    from vibe.app_server import _experiments_service
    _experiments_service.initialize_experiments = no_experiments
    tracing.setup_tracing = lambda config: None

    TelemetryResource.record = lambda self, name, properties=None, *, correlate_last_request=False: None
    TelemetryResource.log = lambda self, event: None

    async def no_client_telemetry_request(self, name, properties, correlate_last_request):
        return None

    TelemetryResource._record = no_client_telemetry_request
