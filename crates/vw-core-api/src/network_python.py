"""Check a connection with installed Vibe's TLS and proxy code; network_python.rs supplies stdin.

No model key, response body, address or exception text is returned. Python 3.14's strict certificate
verification is retained, with the trust store Vibe's config selects. The parent bounds the process as well as this request.
"""
import asyncio
import json
import os
import ssl
import sys

from vibe.utils.http import VibeAsyncHTTPClient, build_ssl_context, configure_ssl_context


async def main():
    request = json.load(sys.stdin)
    for key, value in request["settings"].items():
        os.environ[key] = value
    # The same trust anchors the server pinned from its config: the OS store or the bundled certifi file.
    configure_ssl_context(enable_system_trust_store=bool(request.get("systemTrustStore")))
    try:
        async with VibeAsyncHTTPClient(
            verify=build_ssl_context(), timeout=15, follow_redirects=False
        ) as client:
            async with client.stream("GET", request["url"]):
                pass
        return {"kind": "reached"}
    except Exception as error:
        seen = set()
        while error is not None and id(error) not in seen:
            seen.add(id(error))
            if isinstance(error, ssl.SSLCertVerificationError):
                # A bounded vocabulary, never a peer-controlled subject or address.
                reason = error.verify_message.lower()
                reasons = {
                    "missing authority key identifier": "it lacks an authority key identifier",
                    "missing subject key identifier": "it lacks a subject key identifier",
                    "ca cert does not include key usage extension": "its authority lacks a key usage extension",
                    "basic constraints of ca cert not marked critical": "its authority's basic constraints are not marked critical",
                }
                return {"kind": "certificate", "reason": reasons.get(reason)}
            error = error.__cause__ or error.__context__
        return {"kind": "failed"}


print(json.dumps(asyncio.run(main())))
