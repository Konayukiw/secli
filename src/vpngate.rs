use anyhow::{Context, Result};
use base64::Engine;
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

static REMOTE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*remote\s+(\S+)\s+(\d+)").unwrap());
static PROTO_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*proto\s+(\w+)").unwrap());

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct RawVpnServer {
    #[serde(rename = "#HostName")]
    pub hostname: String,
    #[serde(rename = "IP")]
    pub ip: String,
    #[serde(rename = "Score")]
    pub score: String,
    #[serde(rename = "Ping")]
    pub ping: String,
    #[serde(rename = "Speed")]
    pub speed: String,
    #[serde(rename = "CountryLong")]
    pub country_long: String,
    #[serde(rename = "CountryShort")]
    pub country_short: String,
    #[serde(rename = "NumVpnSessions")]
    pub num_vpn_sessions: String,
    #[serde(rename = "Uptime")]
    pub uptime: String,
    #[serde(rename = "TotalUsers")]
    pub total_users: String,
    #[serde(rename = "TotalTraffic")]
    pub total_traffic: String,
    #[serde(rename = "LogType")]
    pub log_type: String,
    #[serde(rename = "Operator")]
    pub operator: String,
    #[serde(rename = "Message")]
    pub message: String,
    #[serde(rename = "OpenVPN_ConfigData_Base64")]
    pub config_data_b64: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VpnServer {
    pub hostname: String,
    pub ip: String,
    pub score: u64,
    pub ping: Option<u64>,
    pub speed: u64,
    pub country_long: String,
    pub country_short: String,
    pub num_vpn_sessions: u64,
    pub uptime: u64,
    pub total_users: u64,
    pub port: u16,
    pub proto: String,
}

impl VpnServer {
    pub fn speed_mbps(&self) -> f64 {
        self.speed as f64 / 1_000_000.0
    }
}

pub fn fetch_vpn_servers() -> Result<Vec<VpnServer>> {
    let url = "http://www.vpngate.net/api/iphone/";
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(url)
        .send()
        .context("Failed to connect to VPN Gate API")?
        .text()
        .context("Failed to read response body")?;

    parse_vpn_servers(&resp)
}

pub fn parse_vpn_servers(raw_csv: &str) -> Result<Vec<VpnServer>> {
    let filtered_lines: Vec<&str> = raw_csv
        .lines()
        .filter(|line| !line.starts_with('*'))
        .collect();

    let clean_csv = filtered_lines.join("\n");
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(clean_csv.as_bytes());

    let mut servers = Vec::new();
    let b64_engine = base64::engine::general_purpose::STANDARD;

    for result in reader.deserialize() {
        let raw: RawVpnServer = match result {
            Ok(r) => r,
            Err(_) => continue,
        };

        if raw.ip.trim().is_empty() {
            continue;
        }

        let score = raw.score.parse::<u64>().unwrap_or(0);
        let ping = raw.ping.parse::<u64>().ok();
        let speed = raw.speed.parse::<u64>().unwrap_or(0);
        let num_vpn_sessions = raw.num_vpn_sessions.parse::<u64>().unwrap_or(0);
        let uptime = raw.uptime.parse::<u64>().unwrap_or(0);
        let total_users = raw.total_users.parse::<u64>().unwrap_or(0);

        let mut port = 443u16;
        let mut proto = String::from("tcp");

        if !raw.config_data_b64.is_empty() {
            if let Ok(decoded_bytes) = b64_engine.decode(raw.config_data_b64.trim()) {
                let conf = String::from_utf8_lossy(&decoded_bytes);
                if let Some(caps) = PROTO_REGEX.captures(&conf) {
                    if let Some(m) = caps.get(1) {
                        proto = m.as_str().to_lowercase();
                    }
                }
                if let Some(caps) = REMOTE_REGEX.captures(&conf) {
                    if let Some(m) = caps.get(2) {
                        if let Ok(p) = m.as_str().parse::<u16>() {
                            port = p;
                        }
                    }
                }
            }
        }

        servers.push(VpnServer {
            hostname: raw.hostname,
            ip: raw.ip,
            score,
            ping,
            speed,
            country_long: raw.country_long,
            country_short: raw.country_short,
            num_vpn_sessions,
            uptime,
            total_users,
            port,
            proto,
        });
    }

    Ok(servers)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vpn_servers() {
        let sample_csv = "*vpn_servers\n\
#HostName,IP,Score,Ping,Speed,CountryLong,CountryShort,NumVpnSessions,Uptime,TotalUsers,TotalTraffic,LogType,Operator,Message,OpenVPN_ConfigData_Base64\n\
vpn1,1.2.3.4,1000,15,50000000,Japan,JP,5,1000,20,10000,2weeks,op1,msg1,\n\
vpn2,5.6.7.8,2000,-,10000000,United States,US,10,2000,40,20000,2weeks,op2,msg2,\n\
*";
        let servers = parse_vpn_servers(sample_csv).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].ip, "1.2.3.4");
        assert_eq!(servers[0].ping, Some(15));
        assert_eq!(servers[0].country_short, "JP");
        assert_eq!(servers[0].port, 443);

        assert_eq!(servers[1].ip, "5.6.7.8");
        assert_eq!(servers[1].ping, None);
        assert_eq!(servers[1].country_short, "US");
    }
}
