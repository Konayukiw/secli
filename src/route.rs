use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const FOCUS_VPN_METRIC: u32 = 9000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMode {
    Focus,
    Exclude,
}

impl RouteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Exclude => "exclude",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "focus" => Some(Self::Focus),
            "exclude" => Some(Self::Exclude),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteTarget {
    pub spec: String,
    #[allow(dead_code)]
    pub host: String,
    pub ips: Vec<IpAddr>,
    pub cidr: Option<(IpAddr, u8)>,
}


pub fn expand_list_arg(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for v in values {
        for part in v.split(',') {
            let t = part.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    out
}

fn strip_scheme(spec: &str) -> &str {
    let s = spec.trim();
    match s.find("://") {
        Some(pos) => &s[pos + 3..],
        None => s,
    }
}

pub fn extract_host(spec: &str) -> String {
    let mut s = strip_scheme(spec).to_string();
    for sep in ['/', '?', '#'] {
        if let Some(p) = s.find(sep) {
            s.truncate(p);
        }
    }
    if let Some(at) = s.rfind('@') {
        s = s[at + 1..].to_string();
    }
    let s = s.trim().to_string();
    if s.starts_with('[') {
        if let Some(end) = s.find(']') {
            return s[1..end].to_string();
        }
        return s;
    }
    if s.matches(':').count() > 1 {
        return s.trim_end_matches('.').to_string();
    }
    if let Some(colon) = s.rfind(':') {
        let (h, p) = s.split_at(colon);
        let port = &p[1..];
        if !h.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return h.trim_end_matches('.').to_string();
        }
    }
    s.trim_end_matches('.').to_string()
}

pub fn parse_target_spec(spec: &str) -> Result<RouteTarget> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("Empty --focus/--exclude entry");
    }
    let stripped = strip_scheme(spec);
    if let Some((left, right)) = stripped.split_once('/') {
        if !right.contains('/') {
            if let (Ok(net), Ok(len)) = (left.trim().parse::<IpAddr>(), right.trim().parse::<u8>())
            {
                let max = match net {
                    IpAddr::V4(_) => 32,
                    IpAddr::V6(_) => 128,
                };
                if len > max {
                    bail!("Invalid prefix in '{}': /{} (max /{})", spec, len, max);
                }
                return Ok(RouteTarget {
                    spec: spec.to_string(),
                    host: left.trim().to_string(),
                    ips: Vec::new(),
                    cidr: Some((net, len)),
                });
            }
        }
    }
    let host = extract_host(spec);
    if host.is_empty() {
        bail!("Could not parse host from '{}'", spec);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(RouteTarget {
            spec: spec.to_string(),
            host,
            ips: vec![ip],
            cidr: None,
        });
    }
    let addrs = format!("{}:443", host)
        .to_socket_addrs()
        .with_context(|| format!("DNS resolution failed for '{}'", host))?;
    let mut set = BTreeSet::new();
    for a in addrs {
        set.insert(a.ip());
    }
    if set.is_empty() {
        bail!("DNS returned no addresses for '{}'", host);
    }
    let mut ips: Vec<IpAddr> = set.into_iter().collect();
    ips.sort_by_key(|ip| if ip.is_ipv4() { 0u8 } else { 1u8 });
    Ok(RouteTarget {
        spec: spec.to_string(),
        host,
        ips,
        cidr: None,
    })
}

pub fn resolve_targets(specs: &[String]) -> Result<Vec<RouteTarget>> {
    let flat = expand_list_arg(specs);
    if flat.is_empty() {
        bail!("No targets given");
    }
    let mut out = Vec::new();
    for s in &flat {
        out.push(parse_target_spec(s)?);
    }
    Ok(out)
}

#[allow(dead_code)]
pub fn flatten_ips(targets: &[RouteTarget]) -> Vec<IpAddr> {
    let mut set = BTreeSet::new();
    for t in targets {
        for ip in &t.ips {
            set.insert(*ip);
        }
    }
    set.into_iter().collect()
}

pub fn v4_mask_from_len(len: u8) -> String {
    assert!(len <= 32);
    let bits = if len == 0 { 0u32 } else { (!0u32) << (32 - len) };
    format!(
        "{}.{}.{}.{}",
        (bits >> 24) & 255,
        (bits >> 16) & 255,
        (bits >> 8) & 255,
        bits & 255
    )
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    pub dest: String,
    pub mask_or_len: String,
    pub gateway: String,
    pub v6: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultRoute {
    pub gateway: String,
    pub interface_ip: String,
    pub if_index: Option<u32>,
    pub metric: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteState {
    pub mode: String,
    pub specs: Vec<String>,
    pub routes: Vec<RouteEntry>,
    #[serde(default)]
    pub gateway: String,
    #[serde(default)]
    pub pinned: Vec<RouteEntry>,
    #[serde(default)]
    pub vpn_if_index: Option<u32>,
    #[serde(default)]
    pub vpn_if_metric: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct VpnLink {
    pub if_index: u32,
    pub ip: String,
    pub gateway: String,
}

pub fn entry_key(e: &RouteEntry) -> String {
    format!("{}|{}|{}", e.dest, e.mask_or_len, e.gateway)
}

fn state_path() -> Result<PathBuf> {
    let dir = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = dir.join("SoftEther-CLI");
    fs::create_dir_all(&dir).context("Failed to create state dir")?;
    Ok(dir.join("routes.json"))
}

pub fn save_state(state: &RouteState) -> Result<PathBuf> {
    let path = state_path()?;
    let json = serde_json::to_string_pretty(state)?;
    fs::write(&path, json)?;
    Ok(path)
}

pub fn load_state() -> Result<RouteState> {
    let path = state_path()?;
    let data = fs::read_to_string(&path)
        .with_context(|| format!("No route state at {}", path.display()))?;
    Ok(serde_json::from_str(&data)?)
}

#[allow(dead_code)]
pub fn state_path_for_display() -> String {
    state_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

fn run_netsh(args: &[&str]) -> Result<String> {
    let out = Command::new("netsh")
        .args(args)
        .output()
        .context("Failed to run netsh")?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        bail!("netsh {} failed: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

fn run_route(args: &[&str]) -> Result<String> {
    let out = Command::new("route")
        .args(args)
        .output()
        .context("Failed to run route")?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        let lower = text.to_lowercase();
        if lower.contains("already exists")
            || lower.contains("object already exists")
            || lower.contains("already in use")
        {
            return Ok(text);
        }
        bail!("route {} failed: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

pub fn current_default_route() -> Result<DefaultRoute> {
    let text = run_netsh(&["interface", "ipv4", "show", "route"])?;
    let metrics = interface_metrics().unwrap_or_default();
    best_default_from_netsh(&text, &metrics).context("No IPv4 default route found")
}

fn best_default_from_netsh(text: &str, if_metrics: &HashMap<u32, u32>) -> Option<DefaultRoute> {
    let mut best: Option<DefaultRoute> = None;
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 {
            continue;
        }
        let prefix_ok = cols[3] == "0.0.0.0/0";
        let gw_ok = cols[5].parse::<Ipv4Addr>().is_ok();
        if !prefix_ok || !gw_ok {
            continue;
        }
        let if_index: Option<u32> = cols[4].parse().ok();
        let route_metric: u32 = cols[2].parse().unwrap_or(u32::MAX);
        let if_metric = if_index
            .and_then(|i| if_metrics.get(&i).copied())
            .unwrap_or(0);
        let cand = DefaultRoute {
            gateway: cols[5].to_string(),
            interface_ip: String::new(),
            if_index,
            metric: route_metric.saturating_add(if_metric),
        };
        let take = match &best {
            Some(b) => cand.metric < b.metric,
            None => true,
        };
        if take {
            best = Some(cand);
        }
    }
    let mut best = best?;
    if best.interface_ip.is_empty() {
        best.interface_ip = interface_ip_for_gateway(&best.gateway).unwrap_or_default();
    }
    Some(best)
}

fn interface_ip_for_gateway(gateway: &str) -> Option<String> {
    let out = Command::new("route").arg("print").arg("-4").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5 && cols[0] == "0.0.0.0" && cols[2] == gateway {
            return Some(cols[3].to_string());
        }
    }
    None
}

#[allow(dead_code)]
pub fn vpn_default_route() -> Result<DefaultRoute> {
    current_default_route()
}

pub fn vpn_interface_ip(nic_name: &str) -> Result<String> {
    let text = run_netsh(&["interface", "ipv4", "show", "addresses"])?;
    parse_nic_ip(&text, nic_name)
        .with_context(|| format!("Could not find DHCP/IP address for NIC '{}'", nic_name))
}

pub fn interface_metrics() -> Result<HashMap<u32, u32>> {
    let text = run_netsh(&["interface", "ipv4", "show", "interfaces"])?;
    Ok(parse_interface_metrics(&text))
}

fn parse_interface_metrics(text: &str) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        if let (Ok(idx), Ok(met)) = (cols[0].parse::<u32>(), cols[1].parse::<u32>()) {
            out.insert(idx, met);
        }
    }
    out
}

pub fn interface_metric(if_index: u32) -> Result<u32> {
    interface_metrics()?
        .get(&if_index)
        .copied()
        .with_context(|| format!("Interface {} not found", if_index))
}

pub fn set_interface_metric(if_index: u32, metric: u32) -> Result<()> {
    run_netsh(&[
        "interface", "ipv4", "set", "interface",
        &format!("interface={}", if_index),
        &format!("metric={}", metric),
    ])?;
    Ok(())
}

pub fn restore_vpn_metric(state: &RouteState) {
    if let (Some(idx), Some(metric)) = (state.vpn_if_index, state.vpn_if_metric) {
        match set_interface_metric(idx, metric) {
            Ok(()) => println!("Restored VPN interface {} metric to {}", idx, metric),
            Err(e) => eprintln!("  [!] metric restore warning: {:#}", e),
        }
    }
}

pub fn default_gateway_on(if_index: u32) -> Result<String> {
    let text = run_netsh(&["interface", "ipv4", "show", "route"])?;
    parse_default_gateway_on(&text, if_index)
        .with_context(|| format!("No default route on interface {}", if_index))
}

fn parse_default_gateway_on(text: &str, if_index: u32) -> Option<String> {
    text.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let hit = cols.len() >= 6
            && cols[3] == "0.0.0.0/0"
            && cols[4].parse::<u32>().ok() == Some(if_index)
            && cols[5].parse::<Ipv4Addr>().is_ok();
        hit.then(|| cols[5].to_string())
    })
}

fn parse_owner_index(text: &str, ip: &str) -> Option<u32> {
    let host = format!("{}/32", ip);
    text.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5 && cols[3] == host {
            cols[4].parse::<u32>().ok()
        } else {
            None
        }
    })
}

pub fn vpn_link(nic_name: &str) -> Result<VpnLink> {
    let ip = vpn_interface_ip(nic_name)?;
    let text = run_netsh(&["interface", "ipv4", "show", "route"])?;
    let if_index = parse_owner_index(&text, &ip)
        .with_context(|| format!("No interface owns {}", ip))?;
    let gateway = parse_default_gateway_on(&text, if_index)
        .with_context(|| format!("No default route on VPN interface {} yet", if_index))?;
    Ok(VpnLink {
        if_index,
        ip,
        gateway,
    })
}

pub fn wait_vpn_link(nic_name: &str, timeout: Duration) -> Result<VpnLink> {
    let start = Instant::now();
    loop {
        match vpn_link(nic_name) {
            Ok(link) => return Ok(link),
            Err(e) if start.elapsed() >= timeout => return Err(e),
            Err(_) => sleep(Duration::from_millis(500)),
        }
    }
}

fn parse_nic_ip(text: &str, nic_name: &str) -> Option<String> {
    let want = nic_name.trim().to_lowercase();
    let mut current: Option<String> = None;
    let mut current_ip: Option<String> = None;
    let mut found: Option<String> = None;
    let flush = |name: &Option<String>, ip: &Option<String>, found: &mut Option<String>| {
        if found.is_none() {
            if let Some(n) = name {
                if n.trim().to_lowercase().contains(&want)
                    || want.contains(n.trim().to_lowercase().as_str())
                {
                    if let Some(v) = ip {
                        *found = Some(v.clone());
                    }
                }
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = parse_iface_block_header(t) {
            flush(&current, &current_ip, &mut found);
            current = Some(name);
            current_ip = None;
            if found.is_some() {
                break;
            }
            continue;
        }
        if let Some(ip) = parse_ip_line(t) {
            if current_ip.is_none() {
                current_ip = Some(ip);
            }
        }
    }
    flush(&current, &current_ip, &mut found);
    found
}

fn parse_iface_block_header(line: &str) -> Option<String> {
    let t = line;
    let first = t.find('"')?;
    let rest = &t[first + 1..];
    let second = rest.find('"')?;
    let name = rest[..second].trim();
    let lower = t.to_lowercase();
    if lower.contains("interface") || t.contains("インターフェイス") {
        return Some(name.to_string());
    }
    None
}

fn parse_ip_line(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let is_ip = lower.contains("ip address") || line.contains("IP アドレス");
    if !is_ip {
        return None;
    }
    let after = line.split(':').nth(1)?;
    let ip = after.trim().split_whitespace().next()?;
    if ip.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some(ip.to_string());
    }
    None
}

pub fn build_entries(targets: &[RouteTarget], gateway: &str) -> Vec<RouteEntry> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push = |dest: String, mask: String, v6: bool| {
        let key = format!("{}|{}|{}", dest, mask, gateway);
        if seen.insert(key) {
            out.push(RouteEntry {
                dest,
                mask_or_len: mask,
                gateway: gateway.to_string(),
                v6,
            });
        }
    };
    for t in targets {
        if let Some((net, len)) = t.cidr {
            match net {
                IpAddr::V4(v4) => push(v4.to_string(), v4_mask_from_len(len), false),
                IpAddr::V6(_) => eprintln!("  [!] IPv6 CIDR skipped: {}", t.spec),
            }
            continue;
        }
        for ip in &t.ips {
            match ip {
                IpAddr::V4(v4) => push(v4.to_string(), "255.255.255.255".to_string(), false),
                IpAddr::V6(_) => eprintln!("  [!] IPv6 address skipped (need admin + netsh): {}", ip),
            }
        }
    }
    out
}

pub fn add_entry(e: &RouteEntry, metric: u32) -> Result<()> {
    if e.v6 {
        run_netsh(&[
            "interface", "ipv6", "add", "route",
            &format!("{}/{}", e.dest, e.mask_or_len),
            &e.gateway,
            &format!("metric={}", metric),
        ])?;
    } else if e.gateway.to_ascii_lowercase() == "on-link" {
        run_route(&[
            "add", &e.dest, "mask", &e.mask_or_len, &e.dest, "metric",
            &metric.to_string(),
        ])?;
    } else {
        run_route(&[
            "add", &e.dest, "mask", &e.mask_or_len, &e.gateway, "metric",
            &metric.to_string(),
        ])?;
    }
    Ok(())
}

pub fn delete_entry(e: &RouteEntry) -> Result<()> {
    if e.v6 {
        let r = run_netsh(&[
            "interface", "ipv6", "delete", "route",
            &format!("{}/{}", e.dest, e.mask_or_len),
            &e.gateway,
        ]);
        if let Err(err) = r {
            eprintln!("  [!] route cleanup warning: {}", err);
        }
    } else {
        let r = run_route(&["delete", &e.dest]);
        if let Err(err) = r {
            eprintln!("  [!] route cleanup warning: {}", err);
        }
    }
    Ok(())
}

pub fn apply_routes(entries: &[RouteEntry], metric: u32) -> (usize, usize) {
    let mut ok = 0;
    let mut failed = 0;
    for e in entries {
        match add_entry(e, metric) {
            Ok(()) => ok += 1,
            Err(err) => {
                failed += 1;
                eprintln!("  [!] route add failed for {}/{}: {}", e.dest, e.mask_or_len, err);
            }
        }
    }
    (ok, failed)
}

pub fn remove_routes(entries: &[RouteEntry]) {
    for e in entries {
        let _ = delete_entry(e);
    }
}

pub fn stored_state_path() -> PathBuf {
    state_path().unwrap_or_else(|_| PathBuf::from("routes.json"))
}

pub fn require_admin() -> Result<()> {
    let probe = Command::new("netsh")
        .args(["interface", "ipv4", "show", "route"])
        .output()
        .context("Failed to run netsh")?;
    if !probe.status.success() {
        bail!("Route management needs an elevated terminal. Run as an administrator.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTES: &str = "\
Publish  Type      Met  Prefix                    Idx  Gateway/Interface Name
-------  --------  ---  ------------------------  ---  ------------------------
No       Manual    1    0.0.0.0/0                   9  10.211.254.254
No       Manual    0    0.0.0.0/0                   5  192.168.11.1
No       System    256  10.211.0.0/16               9  VPN - VPN Client
No       System    256  10.211.1.23/32              9  VPN - VPN Client
No       System    256  192.168.11.12/32            5  Wi-Fi
";

    const INTERFACES: &str = "\
Idx     Met         MTU          State                Name
---  ----------  ----------  ------------  ---------------------------
  1          75  4294967295  connected     Loopback Pseudo-Interface 1
  9        9000        1500  connected     VPN - VPN Client
  5          35        1500  connected     Wi-Fi
";

    #[test]
    fn test_parse_interface_metrics() {
        let m = parse_interface_metrics(INTERFACES);
        assert_eq!(m.get(&9), Some(&9000));
        assert_eq!(m.get(&5), Some(&35));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn test_best_default_uses_interface_metric() {
        let mut m = parse_interface_metrics(INTERFACES);
        let best = best_default_from_netsh(ROUTES, &m).unwrap();
        assert_eq!(best.gateway, "192.168.11.1");
        assert_eq!(best.metric, 35);
        m.insert(9, 1);
        let best = best_default_from_netsh(ROUTES, &m).unwrap();
        assert_eq!(best.gateway, "10.211.254.254");
        assert_eq!(best.metric, 2);
    }

    #[test]
    fn test_vpn_link_parsers() {
        assert_eq!(parse_owner_index(ROUTES, "10.211.1.23"), Some(9));
        assert_eq!(parse_owner_index(ROUTES, "10.0.0.1"), None);
        assert_eq!(
            parse_default_gateway_on(ROUTES, 9).as_deref(),
            Some("10.211.254.254")
        );
        assert_eq!(parse_default_gateway_on(ROUTES, 3), None);
    }

    #[test]
    fn test_old_state_deserializes() {
        let json = r#"{"mode":"focus","specs":["a.com"],"routes":[],"gateway":"on-link"}"#;
        let st: RouteState = serde_json::from_str(json).unwrap();
        assert!(st.pinned.is_empty());
        assert_eq!(st.vpn_if_index, None);
        assert_eq!(st.vpn_if_metric, None);
    }
}
