//! Which IP addresses the agent browser may reach on the network (navigation policy): global
//! unicast addresses only. Everything in the IANA special-purpose registries that is not globally reachable is
//! refused with a class the refusal text names; addresses that embed an IPv4 address (IPv4-mapped, IPv4-compatible,
//! NAT64 `64:ff9b::/96`, 6to4 `2002::/16`) are judged by the embedded address.
//!
//! Depends on: std only. Called by: `proxy` (every DNS answer and every literal), vw-preview (literal hosts in the
//! navigation hook).
//! Invariants:
//! - Pure and total: every address gets exactly one class; unknown ranges are public.
//! - Loopback is its own class: the caller decides (literal loopback hosts follow the session-port rule; a name that
//!   resolves to loopback is refused like a private answer).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// What kind of address this is, for the policy and the refusal text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IpClass {
    /// Globally reachable unicast.
    Public,
    /// This computer (`127.0.0.0/8`, `::1`).
    Loopback,
    /// A private network (`10/8`, `172.16/12`, `192.168/16`, unique local `fc00::/7`, site-local `fec0::/10`).
    Private,
    /// Link-local (`169.254/16`, `fe80::/10`).
    LinkLocal,
    /// Shared address space used by carrier-grade NAT (`100.64.0.0/10`).
    SharedCgnat,
    /// Multicast or broadcast.
    Multicast,
    /// `0.0.0.0/8`, `::`.
    Unspecified,
    /// Documentation, benchmarking, protocol-assignment and reserved ranges.
    Reserved,
}

impl IpClass {
    /// True only for [`IpClass::Public`].
    #[must_use]
    pub fn is_public(self) -> bool {
        self == Self::Public
    }

    /// Words for a refusal ("a private network address").
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Public => "a public address",
            Self::Loopback => "an address of this computer (loopback)",
            Self::Private => "a private network address",
            Self::LinkLocal => "a link-local address",
            Self::SharedCgnat => "a carrier-grade NAT address (100.64.0.0/10)",
            Self::Multicast => "a multicast or broadcast address",
            Self::Unspecified => "an unspecified address",
            Self::Reserved => "a reserved or documentation address",
        }
    }
}

/// The class of `ip`.
#[must_use]
pub fn classify(ip: IpAddr) -> IpClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

/// The class of an IPv4 address.
#[must_use]
pub fn classify_v4(ip: Ipv4Addr) -> IpClass {
    let [a, b, c, _] = ip.octets();
    match (a, b, c) {
        (0, _, _) => IpClass::Unspecified,
        (127, _, _) => IpClass::Loopback,
        (10, _, _) | (192, 168, _) => IpClass::Private,
        (172, b, _) if (16..=31).contains(&b) => IpClass::Private,
        (169, 254, _) => IpClass::LinkLocal,
        (100, b, _) if (64..=127).contains(&b) => IpClass::SharedCgnat,
        (224..=239, _, _) => IpClass::Multicast,
        _ if ip == Ipv4Addr::BROADCAST => IpClass::Multicast,
        // 192.0.0.0/24 protocol assignments, 192.0.2.0/24 TEST-NET-1, 192.88.99.0/24 6to4 relay anycast,
        // 198.18.0.0/15 benchmarking, 198.51.100.0/24 TEST-NET-2, 203.0.113.0/24 TEST-NET-3, 240.0.0.0/4 reserved.
        (192, 0, 0 | 2)
        | (192, 88, 99)
        | (198, 51, 100)
        | (203, 0, 113)
        | (198, 18 | 19, _)
        | (240..=255, _, _) => IpClass::Reserved,
        _ => IpClass::Public,
    }
}

/// The class of an IPv6 address (embedded IPv4 addresses are judged as IPv4).
#[must_use]
pub fn classify_v6(ip: Ipv6Addr) -> IpClass {
    if let Some(v4) = embedded_v4(ip) {
        return classify_v4(v4);
    }
    let s = ip.segments();
    if ip.is_unspecified() {
        return IpClass::Unspecified;
    }
    if ip.is_loopback() {
        return IpClass::Loopback;
    }
    match s[0] {
        0xfe80..=0xfebf => IpClass::LinkLocal,
        0xfec0..=0xfeff | 0xfc00..=0xfdff => IpClass::Private,
        0xff00..=0xffff => IpClass::Multicast,
        // 2001::/23 IETF protocol assignments (Teredo 2001::/32 included), 2001:db8::/32 documentation.
        0x2001 if s[1] < 0x0200 || s[1] == 0x0db8 => IpClass::Reserved,
        // 3fff::/20 documentation, 100::/64 discard, 64:ff9b:1::/48 local-use NAT64.
        0x3fff if s[1] < 0x1000 => IpClass::Reserved,
        0x0100 if s[1..4] == [0, 0, 0] => IpClass::Reserved,
        0x0064 if s[1] == 0xff9b && s[2] == 1 => IpClass::Reserved,
        // Anything else outside 2000::/3 is unassigned or special.
        x if !(0x2000..=0x3fff).contains(&x) => IpClass::Reserved,
        _ => IpClass::Public,
    }
}

/// The IPv4 address inside an IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible (`::a.b.c.d`), NAT64
/// (`64:ff9b::a.b.c.d`) or 6to4 (`2002:aabb:ccdd::`) address.
#[must_use]
pub fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let o = ip.octets();
    let tail = Ipv4Addr::new(o[12], o[13], o[14], o[15]);
    if s[..5] == [0, 0, 0, 0, 0] && s[5] == 0xffff {
        return Some(tail);
    }
    // IPv4-compatible (deprecated): `::a.b.c.d`, not `::` or `::1`.
    if s[..6] == [0, 0, 0, 0, 0, 0] && (s[6] != 0 || s[7] > 1) {
        return Some(tail);
    }
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return Some(tail);
    }
    if s[0] == 0x2002 {
        return Some(Ipv4Addr::new(o[2], o[3], o[4], o[5]));
    }
    None
}
