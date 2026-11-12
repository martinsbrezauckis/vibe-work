"""Run the installed Vibe billing gateway and account projection against a loopback proxy.
Only synthetic credentials and responses; invoked by the opt-in Rust integration test.
"""
import asyncio
import json
import sys
from importlib.metadata import version
from urllib.parse import urlsplit

from vibe.app_server._account import _budget_view
from vibe.app_server._billing import HttpBillingGateway
from vibe.core.platform_auth import ApiKeyPlatformCredentialProvider


async def main():
    assert version("mistral-vibe") == "3.1.2", "probe the reviewed runtime"
    base, token, mode = sys.argv[1:]
    parsed = urlsplit(base)
    assert parsed.scheme == "http" and parsed.hostname == "127.0.0.1"
    assert parsed.path == "/v1"
    reads = await HttpBillingGateway().read(
        api_base=base,
        credentials=ApiKeyPlatformCredentialProvider.from_api_key(token),
        with_budget=mode != "subscription-only",
    )
    budget = _budget_view(reads)
    print(json.dumps({
        "subscription": reads.subscription.model_dump() if reads.subscription else None,
        "budget": budget.model_dump(mode="json", by_alias=True) if budget else None,
    }))


asyncio.run(main())
