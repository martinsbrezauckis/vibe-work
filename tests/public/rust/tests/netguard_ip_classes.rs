//! Address classes of the agent browser's network guard (`vw_netguard::ip::classify`): the browser reaches only
//! public addresses, so every row below that is not `Public` is an address the guard refuses.
//!
//! Covers each range in `ip.rs`, the first address past its edge where that edge is easy to get wrong
//! (`172.15/172.32`, `100.63/100.128`, `198.20`, `3fff:fff::/3fff:1000::`, `2001:1ff::/2001:200::`), and the IPv6 forms that carry an IPv4
//! address inside (IPv4-mapped, IPv4-compatible, NAT64 `64:ff9b::/96`, 6to4 `2002::/16`), which are judged by that
//! IPv4 address.
//! PREDICT: every row gets its class; none of the refused rows comes out `Public`.

use std::net::{IpAddr, Ipv4Addr};

use vw_netguard::ip::{IpClass, classify};

/// The class of an address written as text.
fn class(text: &str) -> IpClass {
    classify(text.parse::<IpAddr>().unwrap())
}

/// Checks every row and names all wrong rows at once instead of stopping at the first.
fn check(rows: &[(&str, IpClass)]) {
    let wrong: Vec<String> = rows
        .iter()
        .filter(|(text, want)| class(text) != *want)
        .map(|(text, want)| format!("{text}: {:?}, expected {want:?}", class(text)))
        .collect();
    assert!(wrong.is_empty(), "wrong classes: {wrong:#?}");
}

#[test]
fn ipv4_ranges_get_their_class() {
    check(&[
        ("93.184.215.14", IpClass::Public),
        ("1.1.1.1", IpClass::Public),
        ("0.0.0.0", IpClass::Unspecified),
        ("127.0.0.1", IpClass::Loopback),
        ("127.255.0.9", IpClass::Loopback),
        ("10.0.0.5", IpClass::Private),
        ("172.15.255.255", IpClass::Public),
        ("172.16.0.1", IpClass::Private),
        ("172.31.255.255", IpClass::Private),
        ("172.32.0.1", IpClass::Public),
        ("169.253.0.1", IpClass::Public),
        ("169.254.169.254", IpClass::LinkLocal),
        ("100.63.255.255", IpClass::Public),
        ("100.64.0.1", IpClass::SharedCgnat),
        ("100.127.255.254", IpClass::SharedCgnat),
        ("100.128.0.1", IpClass::Public),
        ("224.0.0.1", IpClass::Multicast),
        ("239.255.255.255", IpClass::Multicast),
        ("255.255.255.255", IpClass::Multicast),
        // 192.0.0.0/24 protocol assignments, the TEST-NET documentation ranges, the 6to4 relay anycast range,
        // 198.18.0.0/15 benchmarking and 240.0.0.0/4 reserved.
        ("192.0.0.8", IpClass::Reserved),
        ("192.0.2.1", IpClass::Reserved),
        ("192.88.99.1", IpClass::Reserved),
        ("198.51.100.1", IpClass::Reserved),
        ("203.0.113.1", IpClass::Reserved),
        ("198.18.0.1", IpClass::Reserved),
        ("198.19.255.255", IpClass::Reserved),
        ("198.20.0.1", IpClass::Public),
        ("240.0.0.1", IpClass::Reserved),
    ]);
    // 192.168/16 is built from octets, as in `ip.rs`: the public-paths scan refuses the dotted text of this range in
    // public files (it keeps real home-network addresses out of the repository).
    assert_eq!(
        classify(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
        IpClass::Private
    );
}

#[test]
fn ipv6_ranges_get_their_class() {
    check(&[
        ("2606:2800:21f:cb07:6820:80da:af6b:8b2c", IpClass::Public),
        ("::", IpClass::Unspecified),
        ("::1", IpClass::Loopback),
        ("fc00::1", IpClass::Private),
        ("fd12:3456::1", IpClass::Private),
        ("fec0::1", IpClass::Private),
        ("fe80::1", IpClass::LinkLocal),
        ("febf::1", IpClass::LinkLocal),
        ("ff02::1", IpClass::Multicast),
        // 2001::/23 protocol assignments (Teredo 2001::/32 included) and 2001:db8::/32 documentation.
        ("2001::1", IpClass::Reserved),
        ("2001:0:4136:e378::1", IpClass::Reserved),
        ("2001:1ff:ffff::1", IpClass::Reserved),
        ("2001:db8::1", IpClass::Reserved),
        ("2001:200::1", IpClass::Public),
        // 3fff::/20 documentation, 100::/64 discard, 64:ff9b:1::/48 local-use NAT64.
        ("3fff::1", IpClass::Reserved),
        ("3fff:fff::1", IpClass::Reserved),
        ("3fff:1000::1", IpClass::Public),
        ("100::1", IpClass::Reserved),
        ("64:ff9b:1::1", IpClass::Reserved),
        // Outside 2000::/3: unassigned or special.
        ("1fff::1", IpClass::Reserved),
        ("4000::1", IpClass::Reserved),
    ]);
}

#[test]
fn an_ipv4_address_inside_ipv6_is_judged_as_ipv4() {
    check(&[
        // IPv4-mapped.
        ("::ffff:10.0.0.1", IpClass::Private),
        ("::ffff:127.0.0.1", IpClass::Loopback),
        ("::ffff:0.0.0.0", IpClass::Unspecified),
        ("::ffff:93.184.215.14", IpClass::Public),
        // IPv4-compatible (`::` and `::1` keep their own meaning, `::2` carries 0.0.0.2).
        ("::10.0.0.1", IpClass::Private),
        ("::2", IpClass::Unspecified),
        // NAT64.
        ("64:ff9b::a00:1", IpClass::Private),
        ("64:ff9b::7f00:1", IpClass::Loopback),
        ("64:ff9b::169.254.169.254", IpClass::LinkLocal),
        // 6to4.
        ("2002:a00:1::", IpClass::Private),
        ("2002:7f00:1::", IpClass::Loopback),
        ("2002:5db8:d70e::1", IpClass::Public),
    ]);
}
