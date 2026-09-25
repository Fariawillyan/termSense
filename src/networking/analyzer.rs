//! Extracts networking entities from free text and computes subnets.
//!
//! `quem usa a porta 5432` → port 5432; `testar conexão com db.local` →
//! host `db.local`; `/24` or `192.168.1.0/24` → CIDR. The extracted values
//! become template variables, so knowledge recipes render concrete commands.

use std::net::{Ipv4Addr, Ipv6Addr};

use super::knowledge::port_info;
use crate::knowledge::Vars;

/// A CIDR block; the address is optional for bare prefixes such as `/24`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cidr {
    V4 { addr: Option<Ipv4Addr>, prefix: u8 },
    V6 { addr: Option<Ipv6Addr>, prefix: u8 },
}

/// Networking entities found in a query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetQuery {
    pub ports: Vec<u16>,
    /// A port was stated explicitly (`porta 8080`, `:8080`, `host:8080`).
    pub explicit_port: bool,
    pub hosts: Vec<String>,
    pub urls: Vec<String>,
    pub cidrs: Vec<Cidr>,
    /// Query words that are parameters rather than search terms.
    pub parameters: Vec<String>,
}

impl NetQuery {
    /// Template variables: `port`, `host`, `url`.
    pub fn vars(&self) -> Vars {
        let mut vars = Vars::new();
        if let Some(port) = self.ports.first() {
            vars.set("port", port.to_string());
        }
        if let Some(host) = self.hosts.first() {
            vars.set("host", host.clone());
        }
        if let Some(url) = self.urls.first() {
            vars.set("url", url.clone());
        }
        vars
    }
}

const PORT_WORDS: &[&str] = &["porta", "portas", "port", "ports"];

/// Scans free text for ports, hosts, URLs and CIDR blocks.
pub fn scan(text: &str) -> NetQuery {
    let mut q = NetQuery::default();
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| matches!(c, ',' | ';' | '?' | '!' | '"' | '\'' | '(' | ')'))
        })
        .filter(|w| !w.is_empty())
        .collect();
    for (i, w) in words.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| words[j].to_lowercase());
        if let Some(url) = parse_url(w) {
            q.urls.push(w.to_string());
            if let Some(host) = url.host {
                push_unique(&mut q.hosts, host);
            }
            if let Some(port) = url.port {
                q.ports.push(port);
                q.explicit_port = true;
            }
        } else if let Some(cidr) = parse_cidr(w) {
            q.cidrs.push(cidr);
        } else if w.parse::<Ipv4Addr>().is_ok()
            || (w.contains(':') && w.parse::<Ipv6Addr>().is_ok())
        {
            push_unique(&mut q.hosts, w.to_string());
        } else if let Some(port) = w.strip_prefix(':').and_then(parse_port) {
            q.ports.push(port);
            q.explicit_port = true;
        } else if let Some(port) = parse_port(w) {
            if prev.as_deref().is_some_and(|p| PORT_WORDS.contains(&p)) {
                q.ports.push(port);
                q.explicit_port = true;
            } else if port_info(port).is_some() {
                q.ports.push(port);
                // A bare well-known number is a hint, not a parameter.
                continue;
            } else {
                continue;
            }
        } else if let Some((host, port)) = host_port(w) {
            push_unique(&mut q.hosts, host.to_string());
            q.ports.push(port);
            q.explicit_port = true;
        } else if let Some((_, host)) = w
            .split_once('@')
            .filter(|(u, h)| !u.is_empty() && (is_hostname(h) || h.parse::<Ipv4Addr>().is_ok()))
        {
            push_unique(&mut q.hosts, host.to_string());
        } else if is_hostname(w) {
            push_unique(&mut q.hosts, w.to_string());
        } else {
            continue;
        }
        q.parameters.push(w.to_string());
    }
    q
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn parse_port(s: &str) -> Option<u16> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u16>().ok().filter(|&p| p > 0)
}

fn host_port(s: &str) -> Option<(&str, u16)> {
    let (host, port) = s.rsplit_once(':')?;
    let port = parse_port(port)?;
    (host == "localhost" || host.parse::<Ipv4Addr>().is_ok() || is_hostname(host))
        .then_some((host, port))
}

/// `example.com`, `db.internal`, `localhost` — letters in the last label.
fn is_hostname(s: &str) -> bool {
    if s == "localhost" {
        return true;
    }
    let labels: Vec<&str> = s.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
        && !matches!(
            labels.last().map(|t| t.to_ascii_lowercase()).as_deref(),
            Some(
                "log"
                    | "txt"
                    | "json"
                    | "conf"
                    | "yml"
                    | "yaml"
                    | "sh"
                    | "md"
                    | "rs"
                    | "py"
                    | "js"
                    | "gz"
                    | "tar"
                    | "zip"
                    | "html"
                    | "css"
                    | "toml"
                    | "xml"
                    | "csv"
                    | "ini"
                    | "pem"
                    | "key"
                    | "crt"
            )
        )
}

struct Url {
    host: Option<String>,
    port: Option<u16>,
}

fn parse_url(s: &str) -> Option<Url> {
    let (scheme, rest) = s.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        (h.to_string(), after.strip_prefix(':').and_then(parse_port))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), parse_port(p)),
            None => (authority.to_string(), None),
        }
    };
    Some(Url {
        host: (!host.is_empty()).then_some(host),
        port,
    })
}

/// `/24`, `192.168.1.0/24`, `2001:db8::/32`, `/64` (> 32 means IPv6).
pub fn parse_cidr(s: &str) -> Option<Cidr> {
    let (addr, prefix) = s.rsplit_once('/')?;
    if prefix.is_empty() || prefix.len() > 3 || !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let prefix: u8 = prefix.parse().ok()?;
    if addr.is_empty() {
        return match prefix {
            0..=32 => Some(Cidr::V4 { addr: None, prefix }),
            33..=128 => Some(Cidr::V6 { addr: None, prefix }),
            _ => None,
        };
    }
    if let Ok(v4) = addr.parse::<Ipv4Addr>() {
        return (prefix <= 32).then_some(Cidr::V4 {
            addr: Some(v4),
            prefix,
        });
    }
    if let Ok(v6) = addr.parse::<Ipv6Addr>() {
        return (prefix <= 128).then_some(Cidr::V6 {
            addr: Some(v6),
            prefix,
        });
    }
    None
}

/// Computed facts about an IPv4 block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Net {
    pub prefix: u8,
    pub address: Option<Ipv4Addr>,
    pub mask: Ipv4Addr,
    pub wildcard: Ipv4Addr,
    /// Every address in the block, including network and broadcast.
    pub total: u64,
    /// Addresses assignable to hosts.
    pub usable: u64,
    pub network: Option<Ipv4Addr>,
    pub broadcast: Option<Ipv4Addr>,
    pub first_host: Option<Ipv4Addr>,
    pub last_host: Option<Ipv4Addr>,
}

pub fn ipv4_net(address: Option<Ipv4Addr>, prefix: u8) -> Ipv4Net {
    let prefix = prefix.min(32);
    let mask_bits: u32 = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let total = 1u64 << (32 - u32::from(prefix));
    let usable = usable_hosts(prefix);
    let (network, broadcast, first_host, last_host) = match address {
        Some(a) => {
            let net = u32::from(a) & mask_bits;
            let bcast = net | !mask_bits;
            let (first, last) = match prefix {
                32 => (net, net),
                31 => (net, bcast),
                _ => (net + 1, bcast - 1),
            };
            (
                Some(Ipv4Addr::from(net)),
                (prefix < 31).then(|| Ipv4Addr::from(bcast)),
                Some(Ipv4Addr::from(first)),
                Some(Ipv4Addr::from(last)),
            )
        }
        None => (None, None, None, None),
    };
    Ipv4Net {
        prefix,
        address,
        mask: Ipv4Addr::from(mask_bits),
        wildcard: Ipv4Addr::from(!mask_bits),
        total,
        usable,
        network,
        broadcast,
        first_host,
        last_host,
    }
}

/// Usable hosts: total − network − broadcast, except /31 (point-to-point,
/// RFC 3021: both usable) and /32 (a single host).
pub fn usable_hosts(prefix: u8) -> u64 {
    match prefix.min(32) {
        32 => 1,
        31 => 2,
        p => (1u64 << (32 - u32::from(p))) - 2,
    }
}

/// Network address of an IPv6 block (the host bits zeroed).
pub fn ipv6_network(addr: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let bits = u128::from(addr);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix.min(128)))
    };
    Ipv6Addr::from(bits & mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_explicit_ports() {
        let q = scan("quem usa a porta 8080");
        assert_eq!(q.ports, [8080]);
        assert!(q.explicit_port);
        assert_eq!(q.parameters, ["8080"]);
        assert_eq!(q.vars().get("port"), Some("8080"));

        let q = scan("porta 5432");
        assert_eq!(q.ports, [5432]);
        let q = scan("lsof -i :3000");
        assert_eq!(q.ports, [3000]);
    }

    #[test]
    fn well_known_numbers_are_hints_only() {
        let q = scan("404");
        assert!(q.ports.is_empty());
        let q = scan("22");
        assert_eq!(q.ports, [22]);
        assert!(!q.explicit_port);
        assert!(q.parameters.is_empty());
    }

    #[test]
    fn extracts_hosts_and_urls() {
        let q = scan("testar conexão com api.example.com");
        assert_eq!(q.hosts, ["api.example.com"]);
        let q = scan("curl -v http://localhost:8080/health");
        assert_eq!(q.hosts, ["localhost"]);
        assert_eq!(q.ports, [8080]);
        assert_eq!(q.urls, ["http://localhost:8080/health"]);
        let q = scan("ssh deploy@10.0.0.5");
        assert_eq!(q.hosts, ["10.0.0.5"]);
        let q = scan("ping db.internal:5432");
        assert_eq!((q.hosts[0].as_str(), q.ports[0]), ("db.internal", 5432));
        assert!(scan("grep ERROR app.log").hosts.is_empty());
    }

    #[test]
    fn parses_cidr() {
        assert_eq!(
            parse_cidr("/24"),
            Some(Cidr::V4 {
                addr: None,
                prefix: 24
            })
        );
        assert_eq!(
            parse_cidr("10.0.0.0/8"),
            Some(Cidr::V4 {
                addr: Some(Ipv4Addr::new(10, 0, 0, 0)),
                prefix: 8
            })
        );
        assert!(matches!(
            parse_cidr("/64"),
            Some(Cidr::V6 {
                addr: None,
                prefix: 64
            })
        ));
        assert!(matches!(parse_cidr("2001:db8::/32"), Some(Cidr::V6 { .. })));
        assert_eq!(parse_cidr("10.0.0.0/33"), None);
        assert_eq!(parse_cidr("./logs"), None);
        assert_eq!(parse_cidr("a/24"), None);
    }

    #[test]
    fn subnet_math() {
        let n = ipv4_net(None, 24);
        assert_eq!(n.mask, Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(n.wildcard, Ipv4Addr::new(0, 0, 0, 255));
        assert_eq!((n.total, n.usable), (256, 254));

        let n = ipv4_net(Some(Ipv4Addr::new(192, 168, 1, 77)), 24);
        assert_eq!(n.network, Some(Ipv4Addr::new(192, 168, 1, 0)));
        assert_eq!(n.broadcast, Some(Ipv4Addr::new(192, 168, 1, 255)));
        assert_eq!(n.first_host, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(n.last_host, Some(Ipv4Addr::new(192, 168, 1, 254)));

        let n = ipv4_net(Some(Ipv4Addr::new(172, 16, 5, 4)), 12);
        assert_eq!(n.network, Some(Ipv4Addr::new(172, 16, 0, 0)));
        assert_eq!(n.total, 1 << 20);

        assert_eq!((ipv4_net(None, 32).total, usable_hosts(32)), (1, 1));
        assert_eq!((ipv4_net(None, 31).total, usable_hosts(31)), (2, 2));
        assert_eq!(ipv4_net(None, 0).total, 1 << 32);
        assert_eq!(usable_hosts(8), 16_777_214);
    }

    #[test]
    fn ipv6_network_address() {
        let a: Ipv6Addr = "2001:db8:abcd:12::1".parse().unwrap();
        assert_eq!(
            ipv6_network(a, 48),
            "2001:db8:abcd::".parse::<Ipv6Addr>().unwrap()
        );
    }
}
