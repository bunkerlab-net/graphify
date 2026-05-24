//! IPv4/IPv6 blocklist used by the SSRF guard.
//!
//! Blocks private, loopback, link-local, CGN, NAT64-embedded private,
//! IPv4-mapped private, documentation, and reserved-future address ranges.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::LazyLock;

use ipnet::{Ipv4Net, Ipv6Net};

/// RFC 6598 Shared Address Space (CGN).
#[allow(clippy::expect_used)] // const-evaluated CIDR literal cannot fail at runtime
static CGN_NETWORK: LazyLock<Ipv4Net> =
    LazyLock::new(|| "100.64.0.0/10".parse().expect("static CGN CIDR literal"));
/// RFC 6052 NAT64 Well-Known Prefix.
#[allow(clippy::expect_used)] // const-evaluated CIDR literal cannot fail at runtime
static NAT64_WKP: LazyLock<Ipv6Net> =
    LazyLock::new(|| "64:ff9b::/96".parse().expect("static NAT64 CIDR literal"));

/// Return `true` if `ip` must not be contacted by outbound HTTP, guarding
/// against SSRF to private/reserved addresses.
///
/// For IPv6 addresses inside the NAT64 well-known prefix, the check
/// recursively inspects the embedded IPv4 address.
pub(crate) fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_blocked(v4),
        IpAddr::V6(v6) => {
            if NAT64_WKP.contains(&v6) {
                let octets = v6.octets();
                let embedded = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return ipv4_is_blocked(embedded);
            }
            ipv6_is_blocked(v6)
        }
    }
}

/// Return `true` for IPv4 addresses that are private, loopback, link-local,
/// CGN, or otherwise reserved.
fn ipv4_is_blocked(v4: Ipv4Addr) -> bool {
    if v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_multicast()
        || v4.is_unspecified()
    {
        return true;
    }
    if CGN_NETWORK.contains(&v4) {
        return true;
    }
    let o = v4.octets();
    // RFC 1122: 0.0.0.0/8 "this network". `is_unspecified` only flags the
    // exact `0.0.0.0`; the rest of the range is still reserved.
    if o[0] == 0 {
        return true;
    }
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return true;
    }
    if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
        return true;
    }
    if o[0] >= 240 {
        return true;
    }
    false
}

/// Return `true` for IPv6 addresses that are loopback, unique-local,
/// link-local, IPv4-mapped private, or documentation-only.
fn ipv6_is_blocked(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
        return true;
    }
    let segments = v6.segments();
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    if segments[0..5].iter().all(|s| *s == 0) && segments[5] == 0xffff {
        let v4 = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            (segments[6] & 0xff) as u8,
            (segments[7] >> 8) as u8,
            (segments[7] & 0xff) as u8,
        );
        return ipv4_is_blocked(v4);
    }
    if segments[0] == 0x2001 && segments[1] == 0xdb8 {
        return true;
    }
    // RFC 4380: Teredo (2001:0000::/32) tunnels IPv4 over IPv6.
    if segments[0] == 0x2001 && segments[1] == 0x0000 {
        return true;
    }
    // RFC 3056: 6to4 (2002::/16) embeds an IPv4 address in segments[1..3].
    // Recurse into the embedded IPv4 so a private source v4 is also blocked.
    if segments[0] == 0x2002 {
        let embedded = Ipv4Addr::new(
            (segments[1] >> 8) as u8,
            (segments[1] & 0xff) as u8,
            (segments[2] >> 8) as u8,
            (segments[2] & 0xff) as u8,
        );
        return ipv4_is_blocked(embedded);
    }
    false
}
