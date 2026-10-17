//! Addressing a remote node: parsing `host:port` direct addresses and resolving
//! hostnames, DNS-over-HTTPS first and the device's system resolver second.
//!
//! A direct address is either a literal socket address (`203.0.113.7:11204`,
//! `[2001:db8::1]:11204`), used as-is without any DNS, or `host:port`. A host
//! resolves to every A and AAAA record it has, and each becomes one `IP:port`
//! of the [`EndpointAddr`].
//!
//! Hostnames can never impersonate a node: the endpoint ID is pinned by the
//! caller and checked by the QUIC handshake, so a wrong DNS answer at worst
//! points at the wrong machine, which then fails the handshake.

use anyhow::{Context, Result};
use iroh::dns::{DnsResolver, FallbackMode, NameserverConfig};
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// How long the public DNS-over-HTTPS resolvers get to answer before the
/// system resolver is asked.
pub(crate) const DEFAULT_DOH_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the system resolver gets to answer.
pub(crate) const DEFAULT_SYSTEM_TIMEOUT: Duration = Duration::from_secs(5);

/// The public DNS-over-HTTPS resolvers tried first: Cloudflare (1.1.1.1 and
/// 1.0.0.1) and Quad9 (9.9.9.9). Nameservers are dialled by IP, so the TLS
/// server name is given explicitly for certificate validation.
pub fn default_doh_nameservers() -> Vec<NameserverConfig> {
    let ip = |a: [u8; 4]| IpAddr::from(a);
    vec![
        NameserverConfig::https(ip([1, 1, 1, 1])).with_tls_server_name("cloudflare-dns.com"),
        NameserverConfig::https(ip([1, 0, 0, 1])).with_tls_server_name("cloudflare-dns.com"),
        NameserverConfig::https(ip([9, 9, 9, 9])).with_tls_server_name("dns.quad9.net"),
    ]
}

/// Settings for resolving hostnames in direct addresses.
#[derive(Debug, Clone)]
pub struct HostResolverConfig {
    /// DNS-over-HTTPS nameservers, tried first. Defaults to
    /// [`default_doh_nameservers`]; an empty list skips DoH entirely.
    pub doh_nameservers: Vec<NameserverConfig>,
    /// Time the DoH stage gets per host.
    pub doh_timeout: Duration,
    /// Time the system resolver stage gets per host.
    pub system_timeout: Duration,
}

impl Default for HostResolverConfig {
    fn default() -> Self {
        Self {
            doh_nameservers: default_doh_nameservers(),
            doh_timeout: DEFAULT_DOH_TIMEOUT,
            system_timeout: DEFAULT_SYSTEM_TIMEOUT,
        }
    }
}

/// Resolves hostnames: public DNS-over-HTTPS resolvers first, then, only if
/// none of them gives an answer, the system resolver (which also reads the
/// hosts file, so `localhost` and VPN/enterprise names work).
///
/// Cheap to clone; clones share the resolvers and their caches.
#[derive(Debug, Clone)]
pub struct HostResolver {
    doh: Option<Arc<DnsResolver>>,
    system: Arc<DnsResolver>,
    doh_timeout: Duration,
    system_timeout: Duration,
}

impl HostResolver {
    /// Build both resolvers (no network traffic until the first lookup).
    pub fn new(config: HostResolverConfig) -> Self {
        let doh = if config.doh_nameservers.is_empty() {
            None
        } else {
            // DoH only: no system defaults and no built-in public fallback tier,
            // so the order doh -> system is exactly ours.
            Some(Arc::new(
                DnsResolver::builder()
                    .add_nameserver_configs(config.doh_nameservers)
                    .with_fallback_mode(FallbackMode::Never)
                    .build(),
            ))
        };
        let system = DnsResolver::builder()
            .with_system_defaults()
            .with_fallback_mode(FallbackMode::Never)
            .build();
        Self {
            doh,
            system: Arc::new(system),
            doh_timeout: config.doh_timeout,
            system_timeout: config.system_timeout,
        }
    }

    /// Use these resolvers instead of the built ones (tests).
    #[cfg(test)]
    fn from_resolvers(doh: Option<DnsResolver>, system: DnsResolver) -> Self {
        Self {
            doh: doh.map(Arc::new),
            system: Arc::new(system),
            doh_timeout: Duration::from_secs(2),
            system_timeout: Duration::from_secs(2),
        }
    }

    /// All A and AAAA records of `host`: from DoH if any resolver answers with
    /// at least one address, else from the system resolver. Empty if neither does.
    pub async fn resolve_host(&self, host: &str) -> Vec<IpAddr> {
        if let Some(doh) = &self.doh {
            let ips = lookup(doh, host, self.doh_timeout).await;
            if !ips.is_empty() {
                return ips;
            }
        }
        lookup(&self.system, host, self.system_timeout).await
    }

    /// Turn a parsed target into an [`EndpointAddr`], resolving its hostnames.
    ///
    /// A hostname that does not resolve contributes nothing. It is an error
    /// only if the caller named hostnames, none resolved, and there is no relay
    /// URL or literal address left to try; with none of those given at all,
    /// the endpoint's address lookup (n0 DNS, or the DHT) finds the node.
    pub async fn resolve_target(&self, target: RemoteTarget) -> Result<EndpointAddr> {
        let RemoteTarget {
            id,
            relay,
            sockets,
            hosts,
        } = target;

        let mut tasks = tokio::task::JoinSet::new();
        for (index, (host, port)) in hosts.iter().cloned().enumerate() {
            let resolver = self.clone();
            tasks.spawn(async move { (index, port, resolver.resolve_host(&host).await) });
        }
        let mut resolved = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            resolved.push(joined.context("hostname resolution task failed")?);
        }
        resolved.sort_by_key(|(index, _, _)| *index);

        let mut endpoint_addr = EndpointAddr::from(id);
        if let Some(relay) = relay.clone() {
            endpoint_addr = endpoint_addr.with_relay_url(relay);
        }
        for socket in &sockets {
            endpoint_addr = endpoint_addr.with_ip_addr(*socket);
        }
        let mut resolved_any = false;
        for (_, port, ips) in resolved {
            for ip in ips {
                resolved_any = true;
                endpoint_addr = endpoint_addr.with_ip_addr(SocketAddr::new(ip, port));
            }
        }

        if !hosts.is_empty() && !resolved_any && relay.is_none() && sockets.is_empty() {
            let names: Vec<String> = hosts.iter().map(|(h, p)| format!("{h}:{p}")).collect();
            anyhow::bail!(
                "Could not resolve {} to any address (tried DNS-over-HTTPS, then the system resolver)",
                names.join(", ")
            );
        }
        Ok(endpoint_addr)
    }
}

/// Both address families of `host`, never failing: an error counts as no answer.
async fn lookup(resolver: &DnsResolver, host: &str, timeout: Duration) -> Vec<IpAddr> {
    match resolver.lookup_ipv4_ipv6(host, timeout).await {
        Ok(ips) => {
            let mut ips: Vec<IpAddr> = ips.collect();
            ips.dedup();
            ips
        }
        Err(_) => Vec::new(),
    }
}

/// A remote node as the caller described it, validated but not yet resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTarget {
    pub id: EndpointId,
    pub relay: Option<RelayUrl>,
    /// Literal `IP:port` addresses, used without DNS.
    pub sockets: Vec<SocketAddr>,
    /// `host:port` entries to resolve.
    pub hosts: Vec<(String, u16)>,
}

impl RemoteTarget {
    /// Validate the node ID (hex), the optional relay URL and every direct
    /// address (`IP:port` or `host:port`). Does no DNS.
    pub fn parse(
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        direct_addrs: &[String],
    ) -> Result<Self> {
        let id: EndpointId = remote_node_id_hex
            .parse()
            .context("Failed to parse remote node ID")?;
        let relay = relay_url
            .map(|url| url.parse::<RelayUrl>().context("Invalid relay URL"))
            .transpose()?;

        let mut sockets = Vec::new();
        let mut hosts: Vec<(String, u16)> = Vec::new();
        for entry in direct_addrs {
            match parse_direct_addr(entry)? {
                DirectAddr::Socket(socket) => {
                    if !sockets.contains(&socket) {
                        sockets.push(socket);
                    }
                }
                DirectAddr::Host(host, port) => {
                    if !hosts.contains(&(host.clone(), port)) {
                        hosts.push((host, port));
                    }
                }
            }
        }
        Ok(Self {
            id,
            relay,
            sockets,
            hosts,
        })
    }
}

/// One parsed direct address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectAddr {
    Socket(SocketAddr),
    /// Lowercased hostname and port.
    Host(String, u16),
}

/// Parse `IP:port`, `[IPv6]:port` or `host:port`.
///
/// A hostname is ASCII letters, digits, `-`, `_` and `.` (internationalized
/// names must be given in punycode), at most 253 bytes, with no empty label.
/// The port is 1..=65535.
pub fn parse_direct_addr(entry: &str) -> Result<DirectAddr> {
    if let Ok(socket) = entry.parse::<SocketAddr>() {
        return Ok(DirectAddr::Socket(socket));
    }
    let invalid = |why: &str| {
        anyhow::anyhow!("Invalid direct address {entry:?}: {why} (expected IP:port or host:port)")
    };
    let (host, port) = entry
        .rsplit_once(':')
        .ok_or_else(|| invalid("missing port"))?;
    if host.is_empty() {
        return Err(invalid("empty host"));
    }
    // An unbracketed IPv6 literal (or garbage like "::1") would otherwise be
    // sent to DNS as a hostname.
    if host.contains(':') || host.starts_with('[') {
        return Err(invalid("IPv6 addresses must be written as [addr]:port"));
    }
    if host.len() > 253
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(invalid("not a valid hostname"));
    }
    let name = host.strip_suffix('.').unwrap_or(host);
    if name.is_empty() || name.split('.').any(str::is_empty) {
        return Err(invalid("not a valid hostname"));
    }
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("port must be a number from 1 to 65535"));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| invalid("port must be a number from 1 to 65535"))?;
    if port == 0 {
        return Err(invalid("port must be a number from 1 to 65535"));
    }
    Ok(DirectAddr::Host(host.to_ascii_lowercase(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn target(direct: &[&str]) -> Result<RemoteTarget> {
        let secret = iroh::SecretKey::from_bytes(&[7u8; 32]);
        let direct: Vec<String> = direct.iter().map(|s| s.to_string()).collect();
        RemoteTarget::parse(&secret.public().to_string(), None, &direct)
    }

    #[test]
    fn parses_literals_and_hosts() {
        assert_eq!(
            parse_direct_addr("203.0.113.7:11204").unwrap(),
            DirectAddr::Socket("203.0.113.7:11204".parse().unwrap())
        );
        assert_eq!(
            parse_direct_addr("[2001:db8::1]:443").unwrap(),
            DirectAddr::Socket("[2001:db8::1]:443".parse().unwrap())
        );
        assert_eq!(
            parse_direct_addr("Iroh.Arkavo.net:11204").unwrap(),
            DirectAddr::Host("iroh.arkavo.net".into(), 11204)
        );
        assert_eq!(
            parse_direct_addr("localhost:1").unwrap(),
            DirectAddr::Host("localhost".into(), 1)
        );
        assert_eq!(
            parse_direct_addr("example.com.:65535").unwrap(),
            DirectAddr::Host("example.com.".into(), 65535)
        );
    }

    #[test]
    fn rejects_malformed_direct_addresses() {
        for bad in [
            "",
            "iroh.arkavo.net",
            ":11204",
            "host:",
            "host:0",
            "host:65536",
            "host:-1",
            "host:+80",
            "host:8o",
            "::1",
            "::1:80",
            "[::1]",
            "[foo]:80",
            "a b:80",
            "a..b:80",
            ".:80",
            "host/path:80",
            "user@host:80",
            "http://host:80",
            "1.2.3.4",
        ] {
            assert!(
                parse_direct_addr(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        // The overlong name is rejected too.
        assert!(parse_direct_addr(&format!("{}:80", "a".repeat(254))).is_err());
    }

    #[test]
    fn target_parse_reports_the_bad_entry() {
        let err = target(&["localhost:0"]).unwrap_err();
        assert!(format!("{err:#}").contains("localhost:0"), "{err:#}");
        let err = RemoteTarget::parse("not-hex", None, &[]).unwrap_err();
        assert!(format!("{err:#}").contains("remote node ID"), "{err:#}");
    }

    #[test]
    fn target_parse_separates_and_dedupes() {
        let t = target(&["127.0.0.1:9", "localhost:9", "127.0.0.1:9", "LOCALHOST:9"]).unwrap();
        assert_eq!(t.sockets, vec!["127.0.0.1:9".parse().unwrap()]);
        assert_eq!(t.hosts, vec![("localhost".to_string(), 9)]);
    }

    /// A resolver that answers from a fixed list and counts its queries.
    #[derive(Debug)]
    struct Fixed {
        answers: Vec<IpAddr>,
        queries: Arc<AtomicUsize>,
    }

    impl iroh::dns::Resolver for Fixed {
        fn lookup_ipv4(
            &self,
            _host: String,
        ) -> n0_future_alias::BoxFuture<
            Result<iroh::dns::BoxIter<std::net::Ipv4Addr>, iroh::dns::DnsError>,
        > {
            self.queries.fetch_add(1, Ordering::SeqCst);
            let v4: Vec<_> = self
                .answers
                .iter()
                .filter_map(|ip| match ip {
                    IpAddr::V4(a) => Some(*a),
                    _ => None,
                })
                .collect();
            Box::pin(async move { Ok(Box::new(v4.into_iter()) as iroh::dns::BoxIter<_>) })
        }

        fn lookup_ipv6(
            &self,
            _host: String,
        ) -> n0_future_alias::BoxFuture<
            Result<iroh::dns::BoxIter<std::net::Ipv6Addr>, iroh::dns::DnsError>,
        > {
            let v6: Vec<_> = self
                .answers
                .iter()
                .filter_map(|ip| match ip {
                    IpAddr::V6(a) => Some(*a),
                    _ => None,
                })
                .collect();
            Box::pin(async move { Ok(Box::new(v6.into_iter()) as iroh::dns::BoxIter<_>) })
        }

        fn lookup_txt(
            &self,
            _host: String,
        ) -> n0_future_alias::BoxFuture<
            Result<iroh::dns::BoxIter<iroh::dns::TxtRecordData>, iroh::dns::DnsError>,
        > {
            Box::pin(async move { Ok(Box::new(std::iter::empty()) as iroh::dns::BoxIter<_>) })
        }

        fn clear_cache(&self) {}

        fn reset(&self) -> Box<dyn iroh::dns::Resolver> {
            Box::new(Fixed {
                answers: self.answers.clone(),
                queries: self.queries.clone(),
            })
        }
    }

    mod n0_future_alias {
        pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
    }

    fn fixed(answers: &[&str]) -> (DnsResolver, Arc<AtomicUsize>) {
        let queries = Arc::new(AtomicUsize::new(0));
        let resolver = DnsResolver::custom(Fixed {
            answers: answers.iter().map(|a| a.parse().unwrap()).collect(),
            queries: queries.clone(),
        });
        (resolver, queries)
    }

    fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    #[test]
    fn doh_answer_wins_and_system_is_not_asked() {
        let (doh, doh_queries) = fixed(&["192.0.2.1", "2001:db8::1"]);
        let (system, system_queries) = fixed(&["192.0.2.99"]);
        let resolver = HostResolver::from_resolvers(Some(doh), system);

        let ips = block_on(resolver.resolve_host("iroh.example"));
        assert_eq!(
            ips,
            vec![
                "192.0.2.1".parse::<IpAddr>().unwrap(),
                "2001:db8::1".parse().unwrap()
            ]
        );
        assert_eq!(doh_queries.load(Ordering::SeqCst), 1);
        assert_eq!(system_queries.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn empty_doh_answer_falls_back_to_system() {
        let (doh, doh_queries) = fixed(&[]);
        let (system, system_queries) = fixed(&["192.0.2.99"]);
        let resolver = HostResolver::from_resolvers(Some(doh), system);

        let ips = block_on(resolver.resolve_host("iroh.example"));
        assert_eq!(ips, vec!["192.0.2.99".parse::<IpAddr>().unwrap()]);
        assert_eq!(doh_queries.load(Ordering::SeqCst), 1);
        assert_eq!(system_queries.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn resolve_target_adds_every_record_and_keeps_literals() {
        let (doh, _) = fixed(&["192.0.2.1", "2001:db8::1"]);
        let (system, _) = fixed(&[]);
        let resolver = HostResolver::from_resolvers(Some(doh), system);

        let t = target(&["127.0.0.1:9", "iroh.example:11204"]).unwrap();
        let addr = block_on(resolver.resolve_target(t)).unwrap();
        let mut got: Vec<String> = addr.ip_addrs().map(|a| a.to_string()).collect();
        got.sort();
        assert_eq!(
            got,
            vec!["127.0.0.1:9", "192.0.2.1:11204", "[2001:db8::1]:11204"]
        );
    }

    #[test]
    fn literal_addresses_do_no_dns() {
        let (doh, doh_queries) = fixed(&["192.0.2.1"]);
        let (system, system_queries) = fixed(&["192.0.2.2"]);
        let resolver = HostResolver::from_resolvers(Some(doh), system);

        let t = target(&["127.0.0.1:9", "[::1]:9"]).unwrap();
        let addr = block_on(resolver.resolve_target(t)).unwrap();
        assert_eq!(addr.ip_addrs().count(), 2);
        assert_eq!(doh_queries.load(Ordering::SeqCst), 0);
        assert_eq!(system_queries.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unresolvable_host_fails_unless_something_else_is_left() {
        let (doh, _) = fixed(&[]);
        let (system, _) = fixed(&[]);
        let resolver = HostResolver::from_resolvers(Some(doh), system);

        // Only a hostname, and it resolves to nothing: clear error.
        let err = block_on(resolver.resolve_target(target(&["nowhere.invalid:80"]).unwrap()))
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nowhere.invalid:80"), "{msg}");
        assert!(msg.contains("Could not resolve"), "{msg}");

        // A literal address is left to try: not an error.
        let t = target(&["nowhere.invalid:80", "127.0.0.1:9"]).unwrap();
        assert_eq!(
            block_on(resolver.resolve_target(t))
                .unwrap()
                .ip_addrs()
                .count(),
            1
        );

        // So is a relay URL.
        let secret = iroh::SecretKey::from_bytes(&[7u8; 32]);
        let t = RemoteTarget::parse(
            &secret.public().to_string(),
            Some("https://relay.example.com"),
            &["nowhere.invalid:80".to_string()],
        )
        .unwrap();
        let addr = block_on(resolver.resolve_target(t)).unwrap();
        assert_eq!(addr.relay_urls().count(), 1);

        // No direct addresses at all: untouched, left to address lookup.
        let t = target(&[]).unwrap();
        let addr = block_on(resolver.resolve_target(t)).unwrap();
        assert!(addr.is_empty());
    }
}
