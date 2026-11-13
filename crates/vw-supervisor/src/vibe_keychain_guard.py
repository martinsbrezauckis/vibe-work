"""Write fitting macOS Keychain values through Apple's tool on standard input; keep Vibe's writer as fallback.
On Windows, split values longer than one Credential Manager entry holds into parts.

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


def install_windows_credential_guard():
    """Split values Windows Credential Manager cannot hold in one entry; keep Vibe's own entry for the rest.

    A generic credential holds at most 2560 bytes (1280 UTF-16 characters); a longer write is refused with error
    1783. OAuth tokens are often longer. A long value is stored as numbered parts, each its own credential, and Vibe's
    entry then holds only a marker with the part count and the SHA-256 of the whole value. Reads join and check the
    parts; a missing or changed part reads as no value, so the next sign-in starts fresh. Diagnostics never name items.
    """
    import hashlib
    import importlib.metadata
    import sys

    if sys.platform != "win32" or importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    from vibe.utils import keyring

    limit = 1280
    prefix = "vw-credential-parts:1:"
    current = (keyring._set_password, keyring._get_password, keyring._delete_password)
    set_original, get_original, delete_original = (
        vars(function).get("_vw_windows_credential_original", function) for function in current)

    def part_service(service, username, index):
        return f"{service}:{username}:part{index}"

    def units(text):
        return len(text.encode("utf-16-le", errors="surrogatepass")) // 2

    def split(password):
        parts, part, size = [], [], 0
        for char in password:
            width = units(char)
            if size + width > limit:
                parts.append("".join(part))
                part, size = [], 0
            part.append(char)
            size += width
        parts.append("".join(part))
        return parts

    def stored_parts(service, username):
        """The part count Vibe's entry names, or 0 for a plain or absent entry."""
        try:
            head = get_original(service, username)
        except Exception:
            return 0
        if not head or not head.startswith(prefix):
            return 0
        count = head[len(prefix):].split(":", 1)[0]
        return int(count) if count.isdigit() else 0

    def remove_parts(service, username, start, end):
        for index in range(start, end):
            try:
                delete_original(part_service(service, username, index), username)
            except Exception:
                # A part that is already gone needs no cleanup; the marker no longer names it.
                pass

    def set_password(service, username, password):
        before = stored_parts(service, username)
        if units(password) <= limit:
            set_original(service, username, password)
            remove_parts(service, username, 0, before)
            return
        parts = split(password)
        for index, part in enumerate(parts):
            set_original(part_service(service, username, index), username, part)
        digest = hashlib.sha256(password.encode("utf-8", errors="surrogatepass")).hexdigest()
        set_original(service, username, f"{prefix}{len(parts)}:{digest}")
        remove_parts(service, username, len(parts), before)

    def get_password(service, username):
        head = get_original(service, username)
        if head is None or not head.startswith(prefix):
            return head
        count, _, digest = head[len(prefix):].partition(":")
        if not count.isdigit():
            return None
        parts = [get_original(part_service(service, username, index), username) for index in range(int(count))]
        if any(part is None for part in parts):
            keyring.logger.warning("Windows credential parts: a part is missing; the value reads as absent")
            return None
        value = "".join(parts)
        if hashlib.sha256(value.encode("utf-8", errors="surrogatepass")).hexdigest() != digest:
            keyring.logger.warning("Windows credential parts: the parts do not match; the value reads as absent")
            return None
        return value

    def delete_password(service, username):
        before = stored_parts(service, username)
        delete_original(service, username)
        remove_parts(service, username, 0, before)

    for function, original in zip((set_password, get_password, delete_password),
                                  (set_original, get_original, delete_original)):
        function._vw_windows_credential_original = original
    keyring._set_password = set_password
    keyring._get_password = get_password
    keyring._delete_password = delete_password
