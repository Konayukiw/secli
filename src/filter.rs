use clap::ValueEnum;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use crate::vpngate::VpnServer;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SortKey {
    Ping,
    Country,
    Speed,
    Score,
    Sessions,
}

pub fn sort_servers(servers: &mut [VpnServer], sort_key: SortKey) {
    match sort_key {
        SortKey::Ping => {
            servers.sort_by(|a, b| {
                match (a.ping, b.ping) {
                    (Some(p1), Some(p2)) => p1.cmp(&p2),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });
        }
        SortKey::Country => {
            servers.sort_by(|a, b| {
                a.country_short
                    .cmp(&b.country_short)
                    .then_with(|| a.country_long.cmp(&b.country_long))
                    .then_with(|| a.ping.unwrap_or(u64::MAX).cmp(&b.ping.unwrap_or(u64::MAX)))
            });
        }
        SortKey::Speed => {
            servers.sort_by(|a, b| b.speed.cmp(&a.speed));
        }
        SortKey::Score => {
            servers.sort_by(|a, b| b.score.cmp(&a.score));
        }
        SortKey::Sessions => {
            servers.sort_by(|a, b| a.num_vpn_sessions.cmp(&b.num_vpn_sessions));
        }
    }
}

pub fn filter_servers(
    servers: Vec<VpnServer>,
    country: Option<&str>,
    max_ping: Option<u64>,
    min_speed: Option<f64>,
) -> Vec<VpnServer> {
    servers
        .into_iter()
        .filter(|s| {
            if let Some(c) = country {
                let clow = c.to_lowercase();
                if !s.country_short.to_lowercase().contains(&clow)
                    && !s.country_long.to_lowercase().contains(&clow)
                {
                    return false;
                }
            }
            if let Some(mp) = max_ping {
                match s.ping {
                    Some(p) if p <= mp => {}
                    _ => return false,
                }
            }
            if let Some(ms) = min_speed {
                if s.speed_mbps() < ms {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Check if a server's TCP port is reachable within a short timeout
pub fn check_tcp_reachability(ip: &str, port: u16, timeout: Duration) -> bool {
    let addr_str = format!("{}:{}", ip, port);
    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
        TcpStream::connect_timeout(&addr, timeout).is_ok()
    } else {
        false
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_server(ip: &str, ping: Option<u64>, country: &str, speed: u64) -> VpnServer {
        VpnServer {
            hostname: format!("host-{}", ip),
            ip: ip.to_string(),
            score: 1000,
            ping,
            speed,
            country_long: country.to_string(),
            country_short: country.to_string(),
            num_vpn_sessions: 10,
            uptime: 100,
            total_users: 50,
            port: 443,
            proto: "tcp".to_string(),
        }
    }

    #[test]
    fn test_sort_by_ping() {
        let mut servers = vec![
            create_test_server("1", None, "JP", 100),
            create_test_server("2", Some(50), "US", 100),
            create_test_server("3", Some(10), "KR", 100),
        ];

        sort_servers(&mut servers, SortKey::Ping);

        assert_eq!(servers[0].ip, "3");
        assert_eq!(servers[1].ip, "2");
        assert_eq!(servers[2].ip, "1");
    }

    #[test]
    fn test_sort_by_country() {
        let mut servers = vec![
            create_test_server("1", Some(10), "US", 100),
            create_test_server("2", Some(20), "DE", 100),
            create_test_server("3", Some(30), "JP", 100),
        ];

        sort_servers(&mut servers, SortKey::Country);

        assert_eq!(servers[0].country_short, "DE");
        assert_eq!(servers[1].country_short, "JP");
        assert_eq!(servers[2].country_short, "US");
    }

    #[test]
    fn test_filter_servers() {
        let servers = vec![
            create_test_server("1", Some(10), "JP", 100_000_000),
            create_test_server("2", Some(50), "US", 20_000_000),
            create_test_server("3", Some(100), "JP", 5_000_000),
        ];

        let filtered = filter_servers(servers, Some("JP"), Some(30), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].ip, "1");
    }
}
