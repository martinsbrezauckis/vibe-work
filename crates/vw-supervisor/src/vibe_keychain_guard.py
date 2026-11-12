"""Write fitting macOS Keychain values through Apple's tool on standard input; keep Vibe's writer as fallback.

Installed by the launcher for the audited pin. Items keep the attributes Vibe gives them,
so Vibe's reads stay unchanged. Diagnostics never name items.
"""


def install_macos_keychain_guard():
    import importlib.metadata
    import subprocess
    import sys

    if sys.platform != "darwin" or importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    from vibe.utils import keyring

    current = keyring._set_password
    original = vars(current).get("_vw_macos_keychain_original", current)

    def set_password(service, username, password):
        size = len(password.encode("utf-8", errors="surrogatepass"))
        kind = username.rsplit(":", 1)[-1] if username.startswith("mcp-oauth:") else "other"
        if kind not in {"tokens", "client_info", "fingerprint"}:
            kind = "api_key" if username == "MISTRAL_API_KEY" else "other"

        def report(succeeded):
            outcome = "stored with Vibe's own command" if succeeded else "Vibe's own command failed"
            try:
                keyring.logger.warning(f"macOS Keychain fallback: kind={kind} bytes={size} reason={reason}; {outcome}")
            except Exception:
                # A diagnostic handler must not make a successful sign-in fail.
                pass

        reason = "encoding"
        try:
            data = password.encode("utf-8")
            reason = "item_name"
            if any(char in "\"'\\" or ord(char) < 32 or ord(char) == 127
                   for value in (service, username) for char in value):
                raise ValueError()
            command = (f'add-generic-password -s "{service}" -a "{username}" '
                       f'-X "{data.hex()}" -A\n')
            reason = "oversize"
            # Apple's 4096-byte buffer also needs its NUL terminator. The newline
            # must fit in this read: max bytes = floor((4095 - empty-command bytes)/2).
            if len(command.encode("utf-8")) >= 4096:
                raise ValueError()
            reason = "delete_error"
            try:
                keyring._delete_macos_password(service, username)
            except keyring.PasswordDeleteError:
                pass
            reason = "interactive_error"
            result = subprocess.run(
                ["/usr/bin/security", "-i"], input=command, text=True, encoding="utf-8",
                capture_output=True, check=True, timeout=keyring.SECURITY_TIMEOUT_SECONDS,
            )
            # Piped security -i has no prompt and a successful add has no output.
            reason = "unexpected_output"
            if result.stdout.strip() or result.stderr.strip():
                raise ValueError()
            reason = "readback_error"
            result = keyring._run_security([
                "find-generic-password", "-s", service, "-a", username, "-w",
            ])
            reason = "unexpected_output"
            if result.stderr.strip():
                raise ValueError()
            expected = password if all(32 <= byte < 127 for byte in data) else data.hex()
            reason = "readback_mismatch"
            if result.stdout.removesuffix("\n") != expected:
                raise ValueError()
            return
        except Exception as error:
            if isinstance(error, (subprocess.TimeoutExpired, keyring._KeychainTimeoutError)):
                reason = "timeout"

        # The original is captured before installation, never recursively wrapped.
        # Every guarded failure takes this route exactly once, including timeouts.
        try:
            original(service, username, password)
        except Exception as error:
            report(False)
            if isinstance(error, (subprocess.TimeoutExpired, keyring._KeychainTimeoutError)):
                raise keyring._KeychainTimeoutError(
                    f"macOS Keychain did not answer within {keyring.SECURITY_TIMEOUT_SECONDS}s; "
                    "it may be locked or waiting for approval"
                ) from None
            # Drop the upstream exception chain: it can hold child-process details.
            raise keyring.KeyringError("Can't store password in macOS Keychain") from None
        report(True)

    set_password._vw_macos_keychain_original = original
    keyring._set_password = set_password
