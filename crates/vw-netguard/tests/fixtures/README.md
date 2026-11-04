Local CA DER used only to test saved-certificate decoding in `vw-mistral` and as
an unrelated company root in the proxy platform-store test. It is deliberately
untrusted by the system; it is a CA, not a server leaf.

The proxy tests generate a fresh CA, keys and both server-auth leaves in
`../support/tls.rs` on every test run. The leaves are valid for 397 days starting
yesterday. The passing leaf covers localhost and 127.0.0.1; the wrong-name leaf
has neither name and shares the same issuer. Linux uses ring-backed rcgen and
DER/PKCS#8 with rustls; its isolated platform-store subprocess writes the fresh
CA PEM to a scratch directory. Windows/macOS use pure-Rust RSA signing and
in-memory PKCS#12 identities (password `test-fixture`) with native-tls.
