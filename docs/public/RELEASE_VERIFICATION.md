# Verifying a Vibe Work release

Every release on [GitHub Releases](https://github.com/martinsbrezauckis/vibe-work/releases) is built from the commit
its tag points to. The installers are built and signed on the maintainer's machines (the signing keys never leave
them); the repository's CI builds and tests the same source on Windows, macOS and Linux but uploads nothing.

## Checksums

Each release has a `SHA256SUMS` file listing every asset. Check a download against it:

```sh
sha256sum -c SHA256SUMS --ignore-missing        # Linux
shasum -a 256 -c SHA256SUMS --ignore-missing    # macOS
```

On Windows (PowerShell): `Get-FileHash .\Vibe-Work_<version>_x64-setup.exe -Algorithm SHA256` and compare the value
with the line in `SHA256SUMS`.

## Windows: Authenticode signature

The installer is signed by **U1C** (Lielvārde, Latvia) with an RFC 3161 timestamp, so the signature stays valid after
the signing certificate expires.

```powershell
Get-AuthenticodeSignature .\Vibe-Work_<version>_x64-setup.exe | Format-List Status, SignerCertificate, TimeStamperCertificate
```

`Status` must be `Valid` and the signer's subject must start with `CN=U1C`.

## macOS: Developer ID and notarization

The app and the `.dmg` are signed with the Developer ID of **Martins Brezauckis (team 4M329JW6R4)** and notarized by
Apple; the notarization ticket is stapled, so the check works offline.

```sh
spctl -a -vvv -t open --context context:primary-signature Vibe_Work_<version>_arm64.dmg
# after opening the .dmg:
spctl -a -vvv "/Volumes/Vibe Work/Vibe Work.app"
codesign -dvv "/Volumes/Vibe Work/Vibe Work.app" 2>&1 | grep -E 'Authority|TeamIdentifier'
```

Both `spctl` checks must say `accepted` and `source=Notarized Developer ID`.

## Linux

The `.AppImage` and `.deb` are not signed with an operating-system mechanism; check them against `SHA256SUMS`.

## Updates

The app updates itself (Windows, macOS, AppImage) only from the release channel's `latest.json`, and installs an
update only when its signature verifies against the public key built into the app (minisign key `DAE2633070D0BF4A`):

```
untrusted comment: minisign public key: DAE2633070D0BF4A
RWRKv9BwMGPi2mz79c7PCU8rZVrQReQmKriKANljN+RWxvXflA1TPKDj
```

Each update file has a matching `.sig` asset in the same release. The list of Vibe versions the app may install
(`vibe-support.json`) is signed with the same key. The `.deb` is not updated by the app: install the next `.deb`.

Before downloading, the app checks the offered version's system requirements. `latest.json` records
`minimumSystemVersion` for macOS and Windows, and `minimumGlibc` for Linux. Windows releases require at least
Windows 10 1809 (build `10.0.17763`); the macOS bundle requires 11.0 or later. The Linux minimum is measured from
the AppImage's programs for each build. If the computer is too old, Settings > Updates shows the required version
and does not download. An unreadable or ambiguous minimum makes the update check fail.

## Reporting a problem

If a check fails, do not run the file. Report it as described in `SECURITY.md`.
