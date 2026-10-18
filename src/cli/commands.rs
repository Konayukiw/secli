use anyhow::{bail, Context, Result};
use clap::Parser;
use std::thread::sleep;
use std::time::Duration;
use tabled::settings::Style;
use tabled::Table;

use super::types::{
    Cli, Commands, ServerRow, RouteAction, DEFAULT_ACCOUNT_NAME, DEFAULT_HUB, DEFAULT_USER,
};
use crate::filter::{check_tcp_reachability, filter_servers, sort_servers, SortKey};
use crate::installer;
use crate::softether::VpnCmd;
use crate::route;
use crate::vpngate::{fetch_vpn_servers, VpnServer};

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let Cli {
        command,
        auto,
        sort: g_sort,
        country: g_country,
        max_ping: g_max_ping,
        min_speed: g_min_speed,
        nic: g_nic,
        focus: g_focus,
        exclude: g_exclude,
    } = cli;

    match command {
        Some(Commands::Install) => {
            installer::install_self()?;
        }
        Some(Commands::List {
            sort,
            country,
            max_ping,
            min_speed,
            limit,
        }) => {
            cmd_list(sort, country.as_deref(), max_ping, min_speed, limit)?;
        }
        Some(Commands::Connect {
            ip,
            port,
            nic,
            sort,
            country,
            max_ping,
            min_speed,
            retry_count,
            focus,
            exclude,
        }) => {
            let mut f = g_focus;
            f.extend(focus);
            let mut e = g_exclude;
            e.extend(exclude);
            if let Some(target_ip) = ip {
                cmd_connect_direct(&target_ip, port, &nic, f, e)?;
            } else {
                cmd_auto_connect(
                    sort,
                    country.as_deref(),
                    max_ping,
                    min_speed,
                    &nic,
                    retry_count,
                    f,
                    e,
                )?;
            }
        }
        Some(Commands::Route { action }) => {
            cmd_route(action)?;
        }
        Some(Commands::Disconnect { account }) => {
            cmd_disconnect(&account)?;
        }
        Some(Commands::Status { account }) => {
            cmd_status(&account)?;
        }
        Some(Commands::Accounts) => {
            let out = VpnCmd::list_accounts()?;
            println!("{}", out);
        }
        Some(Commands::Nics) => {
            let nics = VpnCmd::get_nics()?;
            println!("Virtual Network Adapters (NICs):");
            if nics.is_empty() {
                println!("  (None found)");
            } else {
                for nic in nics {
                    println!("  - {}", nic);
                }
            }
        }
        None => {
            if auto {
                cmd_auto_connect(
                    g_sort,
                    g_country.as_deref(),
                    g_max_ping,
                    g_min_speed,
                    &g_nic,
                    5,
                    g_focus,
                    g_exclude,
                )?;
            } else {
                println!("Fetching VPN Gate server list...\n");
                cmd_list(
                    g_sort,
                    g_country.as_deref(),
                    g_max_ping,
                    g_min_speed,
                    20,
                )?;
                println!(
                    "\nTip: Run 'secli connect' or 'secli -a' to auto-connect to the best server."
                );
                println!("     Run 'secli help' to see all commands and options.");
            }
        }
    }

    Ok(())
}

fn cmd_list(
    sort: SortKey,
    country: Option<&str>,
    max_ping: Option<u64>,
    min_speed: Option<f64>,
    limit: usize,
) -> Result<()> {
    println!("Fetching available servers from VPN Gate...");
    let servers = fetch_vpn_servers()?;
    println!("Fetched {} total servers.", servers.len());

    let mut filtered = filter_servers(servers, country, max_ping, min_speed);
    sort_servers(&mut filtered, sort);

    let display_count = filtered.len().min(limit);
    let rows: Vec<ServerRow> = filtered
        .iter()
        .take(display_count)
        .map(ServerRow::from)
        .collect();

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    println!("{}", table);
    println!(
        "Showing {} of {} matching servers (sorted by {:?}).",
        display_count,
        filtered.len(),
        sort
    );

    Ok(())
}

fn cmd_connect_direct(
    ip: &str,
    custom_port: Option<u16>,
    nic_name: &str,
    focus: Vec<String>,
    exclude: Vec<String>,
) -> Result<()> {
    let mut plan = RoutePlan::prepare(focus, exclude, nic_name)?;
    let nic = VpnCmd::ensure_nic(nic_name)?;

    let port = if let Some(p) = custom_port {
        p
    } else {
        println!("Checking server info for {}...", ip);
        if let Ok(servers) = fetch_vpn_servers() {
            servers
                .iter()
                .find(|s| s.ip == ip)
                .map(|s| s.port)
                .unwrap_or(443)
        } else {
            443
        }
    };

    println!(
        "Connecting to {} on port {} using NIC '{}'...",
        ip, port, nic
    );
    plan.pin_vpn_server_route(ip)?;
    VpnCmd::connect(
        DEFAULT_ACCOUNT_NAME,
        ip,
        port,
        DEFAULT_HUB,
        DEFAULT_USER,
        &nic,
        plan.no_track(),
    )?;

    wait_for_connection(DEFAULT_ACCOUNT_NAME, ip, port, 10)?;
    plan.finish_after_connect()?;
    Ok(())
}


fn cmd_auto_connect(
    sort: SortKey,
    country: Option<&str>,
    max_ping: Option<u64>,
    min_speed: Option<f64>,
    nic_name: &str,
    retry_count: usize,
    focus: Vec<String>,
    exclude: Vec<String>,
) -> Result<()> {
    let mut plan = RoutePlan::prepare(focus, exclude, nic_name)?;
    let nic = VpnCmd::ensure_nic(nic_name)?;

    println!("Fetching available servers from VPN Gate...");
    let servers = fetch_vpn_servers()?;
    let mut filtered = filter_servers(servers, country, max_ping, min_speed);

    if filtered.is_empty() {
        eprintln!("No servers matched the given criteria.");
        return Ok(());
    }

    sort_servers(&mut filtered, sort);
    println!(
        "Found {} matching servers. Prioritizing by {:?}...",
        filtered.len(),
        sort
    );

    let candidates: Vec<VpnServer> = filtered.into_iter().take(retry_count.max(10)).collect();

    let mut connected = false;
    for (idx, server) in candidates.iter().enumerate() {
        println!(
            "\n[{}/{}] Trying {}:{} (Country: {}, Ping: {} ms, Speed: {:.1} Mbps)...",
            idx + 1,
            candidates.len().min(retry_count),
            server.ip,
            server.port,
            server.country_short,
            server.ping
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string()),
            server.speed_mbps()
        );

        print!("  Checking TCP port reachability... ");
        if !check_tcp_reachability(&server.ip, server.port, Duration::from_secs(2)) {
            println!("Unreachable (timeout). Skipping.");
            continue;
        }
        println!("Reachable!");

        print!("  Initiating VPN connection... ");
        plan.pin_vpn_server_route(&server.ip)?;
        if let Err(e) = VpnCmd::connect(
            DEFAULT_ACCOUNT_NAME,
            &server.ip,
            server.port,
            DEFAULT_HUB,
            DEFAULT_USER,
            &nic,
            plan.no_track(),
        ) {
            println!("Failed: {}", e);
            continue;
        }
        println!("Requested.");

        match wait_for_connection(DEFAULT_ACCOUNT_NAME, &server.ip, server.port, 8) {
            Ok(_) => {
                connected = true;
                if let Err(e) = plan.finish_after_connect() {
                    eprintln!("  [!] route setup failed: {:#}", e);
                }
                break;
            }
            Err(e) => {
                println!("  Connection failed: {}", e);
                let _ = VpnCmd::disconnect(DEFAULT_ACCOUNT_NAME);
            }
        }

        if idx + 1 >= retry_count {
            break;
        }
    }

    if connected {
        println!("\n==> Successfully connected to VPN Gate!");
        println!("Run 'secli status' to check details or 'secli disconnect' to disconnect.");
    } else {
        eprintln!("\nFailed to establish connection after trying top priority servers.");
        eprintln!("Try selecting another country or re-running with a different sort key.");
    }

    Ok(())
}

fn wait_for_connection(
    account_name: &str,
    ip: &str,
    port: u16,
    timeout_secs: u64,
) -> Result<()> {
    print!("  Waiting for session establishment");
    for _ in 0..timeout_secs {
        sleep(Duration::from_secs(1));
        print!(".");
        if VpnCmd::is_connected(account_name) {
            println!(" Connected!");
            println!("  Session successfully established with {}:{}!", ip, port);
            return Ok(());
        }
    }
    println!();
    anyhow::bail!("Connection timed out waiting for session establishment")
}

fn cmd_disconnect(account: &str) -> Result<()> {
    println!("Disconnecting VPN session for '{}'...", account);
    let state = route::load_state().ok();
    if let Some(st) = &state {
        route::restore_vpn_metric(st);
    }
    VpnCmd::disconnect(account)?;
    if let Some(st) = &state {
        println!("Removing {} route(s)...", st.routes.len() + st.pinned.len());
        route::remove_routes(&st.routes);
        route::remove_routes(&st.pinned);
        let _ = std::fs::remove_file(route::stored_state_path());
    }
    sleep(Duration::from_millis(500));
    VpnCmd::delete_account(account)?;
    println!("VPN disconnected and connection setting cleared.");
    Ok(())
}

fn cmd_status(account: &str) -> Result<()> {
    match VpnCmd::get_status(account) {
        Ok(status) => {
            println!("=== VPN Connection Status for '{}' ===", account);
            println!("{}", status);
        }
        Err(e) => {
            println!("Account '{}' is offline or not found: {}", account, e);
        }
    }
    Ok(())
}

struct RoutePlan {
    focus_specs: Vec<String>,
    focus: Vec<route::RouteTarget>,
    exclude: Vec<route::RouteTarget>,
    original_gateway: Option<String>,
    nic_name: String,
    pinned: Vec<route::RouteEntry>,
}

impl RoutePlan {
    fn prepare(focus: Vec<String>, exclude: Vec<String>, nic_name: &str) -> Result<Self> {
        let focus_flat = route::expand_list_arg(&focus);
        let exclude_flat = route::expand_list_arg(&exclude);
        if !focus_flat.is_empty() && !exclude_flat.is_empty() {
            bail!("--focus and --exclude cannot be used together");
        }
        if focus_flat.is_empty() && exclude_flat.is_empty() {
            return Ok(Self {
                focus_specs: Vec::new(),
                focus: Vec::new(),
                exclude: Vec::new(),
                original_gateway: None,
                nic_name: nic_name.to_string(),
                pinned: Vec::new(),
            });
        }
        route::require_admin()?;
        let original = route::current_default_route()?;
        println!(
            "Original default gateway: {} (metric {})",
            original.gateway, original.metric
        );
        if !focus_flat.is_empty() {
            let targets = route::resolve_targets(&focus_flat)?;
            print_focus_summary(&targets);
            Ok(Self {
                focus_specs: focus_flat,
                focus: targets,
                exclude: Vec::new(),
                original_gateway: Some(original.gateway),
                nic_name: nic_name.to_string(),
                pinned: Vec::new(),
            })
        } else {
            let targets = route::resolve_targets(&exclude_flat)?;
            print_exclude_summary(&targets);
            let entries = route::build_entries(&targets, &original.gateway);
            println!(
                "Pinning {} route(s) via original gateway {} ...",
                entries.len(),
                original.gateway
            );
            let (ok, failed) = route::apply_routes(&entries, 1);
            println!("  routes added: {} ok, {} failed", ok, failed);
            if failed > 0 && ok == 0 {
                anyhow::bail!("No exclude routes could be added. Try running as Administrator.");
            }
            let state = route::RouteState {
                mode: route::RouteMode::Exclude.as_str().to_string(),
                specs: exclude_flat,
                routes: entries,
                gateway: original.gateway.clone(),
                pinned: Vec::new(),
                vpn_if_index: None,
                vpn_if_metric: None,
            };
            let path = route::save_state(&state)?;
            println!("  saved route state: {}", path.display());
            Ok(Self {
                focus_specs: Vec::new(),
                focus: Vec::new(),
                exclude: targets,
                original_gateway: Some(original.gateway),
                nic_name: nic_name.to_string(),
                pinned: Vec::new(),
            })
        }
    }

    fn no_track(&self) -> bool {
        !self.focus_specs.is_empty()
    }

    fn pin_vpn_server_route(&mut self, server_ip: &str) -> Result<()> {
        if self.focus_specs.is_empty() && self.exclude.is_empty() {
            return Ok(());
        }
        let gw = match &self.original_gateway {
            Some(g) => g.clone(),
            None => return Ok(()),
        };
        if server_ip.parse::<std::net::IpAddr>().is_ok() && !server_ip.contains(':') {
            let entry = route::RouteEntry {
                dest: server_ip.to_string(),
                mask_or_len: "255.255.255.255".to_string(),
                gateway: gw.clone(),
                v6: false,
            };
            println!("Pinning VPN server {} via {} ...", server_ip, gw);
            let (ok, failed) = route::apply_routes(std::slice::from_ref(&entry), 1);
            if failed > 0 {
                anyhow::bail!(
                    "Failed to pin VPN server route (need Administrator?): {} ok, {} failed",
                    ok,
                    failed
                );
            }
            let key = route::entry_key(&entry);
            if !self.pinned.iter().any(|r| route::entry_key(r) == key) {
                self.pinned.push(entry);
            }
            if !self.exclude.is_empty() {
                if let Ok(mut st) = route::load_state() {
                    st.pinned = self.pinned.clone();
                    let _ = route::save_state(&st);
                }
            }
        }
        Ok(())
    }

    fn finish_after_connect(&self) -> Result<()> {
        if self.focus_specs.is_empty() {
            if !self.exclude.is_empty() {
                println!("Exclude mode active: listed domains stay on original connection.");
                println!("Existing TCP was preserved (no reconnect).");
            }
            return Ok(());
        }
        let targets = match route::resolve_targets(&self.focus_specs) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  [!] Post-VPN DNS re-resolve failed: {:#}", e);
                eprintln!("  [!] Falling back to pre-VPN addresses.");
                self.focus.clone()
            }
        };
        let link = route::wait_vpn_link(&self.nic_name, Duration::from_secs(20))
            .context("VPN interface did not receive a default gateway")?;
        let original_metric = original_vpn_metric(link.if_index)?;
        println!(
            "VPN interface {} ({}), gateway {}: metric {} -> {}",
            link.if_index,
            link.ip,
            link.gateway,
            original_metric,
            route::FOCUS_VPN_METRIC
        );
        route::set_interface_metric(link.if_index, route::FOCUS_VPN_METRIC)?;
        let entries = route::build_entries(&targets, &link.gateway);
        let state = route::RouteState {
            mode: route::RouteMode::Focus.as_str().to_string(),
            specs: self.focus_specs.clone(),
            routes: entries.clone(),
            gateway: link.gateway.clone(),
            pinned: self.pinned.clone(),
            vpn_if_index: Some(link.if_index),
            vpn_if_metric: Some(original_metric),
        };
        let path = route::save_state(&state)?;
        println!("Routing {} route(s) via VPN gateway {} ...", entries.len(), link.gateway);
        let (ok, failed) = route::apply_routes(&entries, 1);
        println!("  routes added: {} ok, {} failed", ok, failed);
        println!("  saved route state: {}", path.display());
        if failed > 0 && ok == 0 {
            bail!("No focus routes could be added. Run 'secli disconnect' to restore the VPN metric.");
        }
        println!("Focus mode active: only listed domains use VPN, the rest stays direct.");
        Ok(())
    }
}

fn original_vpn_metric(if_index: u32) -> Result<u32> {
    if let Ok(st) = route::load_state() {
        if st.vpn_if_index == Some(if_index) {
            if let Some(m) = st.vpn_if_metric {
                return Ok(m);
            }
        }
    }
    let current = route::interface_metric(if_index)?;
    if current == route::FOCUS_VPN_METRIC {
        eprintln!("  [!] VPN interface metric is already {}, assuming original 1", current);
        return Ok(1);
    }
    Ok(current)
}

fn print_focus_summary(targets: &[route::RouteTarget]) {
    println!("Focus mode: Only these go via VPN, everything else stays direct:");
    for t in targets {
        if let Some((net, len)) = t.cidr {
            println!("  - {} -> {}/{}", t.spec, net, len);
        } else {
            let ips: Vec<String> = t.ips.iter().map(|ip| ip.to_string()).collect();
            println!("  - {} -> {}", t.spec, ips.join(", "));
        }
    }
    println!("NOTE: Focus applies to NEW connections after VPN connects.");
}

fn print_exclude_summary(targets: &[route::RouteTarget]) {
    println!("Exclude mode: These stay on original connection:");
    for t in targets {
        if let Some((net, len)) = t.cidr {
            println!("  - {} -> {}/{}", t.spec, net, len);
        } else {
            let ips: Vec<String> = t.ips.iter().map(|ip| ip.to_string()).collect();
            println!("  - {} -> {}", t.spec, ips.join(", "));
        }
    }
}

fn cmd_route(action: RouteAction) -> Result<()> {
    match action {
        RouteAction::Status => {
            match route::load_state() {
                Ok(st) => {
                    println!("Route mode: {}", st.mode);
                    println!("Targets:");
                    for s in &st.specs {
                        println!("  - {}", s);
                    }
                    println!("Routes ({}):", st.routes.len());
                    for r in &st.routes {
                        println!("  - {}/{} via {}", r.dest, r.mask_or_len, r.gateway);
                    }
                    if !st.pinned.is_empty() {
                        println!("VPN server pins ({}):", st.pinned.len());
                        for r in &st.pinned {
                            println!("  - {}/{} via {}", r.dest, r.mask_or_len, r.gateway);
                        }
                    }
                    if let (Some(idx), Some(orig)) = (st.vpn_if_index, st.vpn_if_metric) {
                        let now = route::interface_metric(idx)
                            .map(|m| m.to_string())
                            .unwrap_or_else(|_| "?".to_string());
                        println!("VPN interface {}: metric {} (restores to {})", idx, now, orig);
                    }
                }
                Err(_) => {
                    println!("No route state saved yet.");
                    println!("State file: {}", route::stored_state_path().display());
                }
            }
            match route::current_default_route() {
                Ok(d) => println!("Current default gateway: {} (metric {})", d.gateway, d.metric),
                Err(e) => println!("Could not read default route: {:#}", e),
            }
        }
        RouteAction::Refresh => {
            refresh_routes()?;
        }
        RouteAction::Watch { interval } => {
            let secs = interval.max(10);
            println!("Watching routes every {}s (Ctrl + C to stop)...", secs);
            loop {
                if let Err(e) = refresh_routes() {
                    eprintln!("[!] refresh failed: {:#}", e);
                }
                sleep(Duration::from_secs(secs));
            }
        }
        RouteAction::Clear => {
            route::require_admin()?;
            match route::load_state() {
                Ok(st) => {
                    println!("Removing {} route(s)...", st.routes.len());
                    route::remove_routes(&st.routes);
                    route::restore_vpn_metric(&st);
                    if st.pinned.is_empty() {
                        let _ = std::fs::remove_file(route::stored_state_path());
                    } else {
                        let kept = route::RouteState {
                            mode: st.mode.clone(),
                            specs: Vec::new(),
                            routes: Vec::new(),
                            gateway: st.gateway.clone(),
                            pinned: st.pinned.clone(),
                            vpn_if_index: None,
                            vpn_if_metric: None,
                        };
                        route::save_state(&kept)?;
                        println!(
                            "Kept {} VPN server pin(s) until 'secli disconnect'.",
                            st.pinned.len()
                        );
                    }
                    println!("Routes cleared.");
                }
                Err(_) => println!("No saved route state, nothing to clear."),
            }
        }
    }
    Ok(())
}

fn refresh_routes() -> Result<()> {
    refresh_routes_inner(None)?;
    Ok(())
}

fn refresh_routes_inner(nic_hint: Option<&str>) -> Result<()> {
    route::require_admin()?;
    let st = route::load_state()?;
    let mode = route::RouteMode::parse(&st.mode)
        .ok_or_else(|| anyhow::anyhow!("Unknown route mode in state: {}", st.mode))?;
    let targets = route::resolve_targets(&st.specs)?;
    let gateway = match mode {
        route::RouteMode::Focus if st.vpn_if_index.is_some() => {
            let idx = st.vpn_if_index.unwrap_or_default();
            route::default_gateway_on(idx).unwrap_or_else(|e| {
                eprintln!("  [!] {:#}, keeping {}", e, st.gateway);
                st.gateway.clone()
            })
        }
        route::RouteMode::Focus => {
            let live = route::current_default_route()?;
            if live.gateway != st.gateway && !st.gateway.is_empty() && st.gateway != "on-link" {
                live.gateway
            } else if st.gateway.is_empty() {
                live.gateway
            } else {
                st.gateway.clone()
            }
        }
        route::RouteMode::Exclude => {
            if st.gateway.is_empty() {
                route::current_default_route()?.gateway
            } else {
                st.gateway.clone()
            }
        }
    };
    let _ = nic_hint;
    let fresh = route::build_entries(&targets, &gateway);
    let fresh_keys: std::collections::BTreeSet<String> = fresh
        .iter()
        .map(|r| format!("{}|{}|{}", r.dest, r.mask_or_len, r.gateway))
        .collect();
    let mut to_delete = Vec::new();
    for r in &st.routes {
        let key = format!("{}|{}|{}", r.dest, r.mask_or_len, r.gateway);
        if r.gateway != gateway || !fresh_keys.contains(&key) {
            to_delete.push(r.clone());
        }
    }
    if st.routes.iter().any(|r| r.gateway != gateway) {
        println!("Gateway changed. Rebuilding routes via {} ...", gateway);
    }
    route::remove_routes(&to_delete);
    let (ok, failed) = route::apply_routes(&fresh, 1);
    let new_state = route::RouteState {
        mode: mode.as_str().to_string(),
        specs: st.specs.clone(),
        routes: fresh,
        gateway,
        pinned: st.pinned.clone(),
        vpn_if_index: st.vpn_if_index,
        vpn_if_metric: st.vpn_if_metric,
    };
    let path = route::save_state(&new_state)?;
    println!(
        "Refreshed {} route(s): {} ok, {} failed. State: {}",
        new_state.routes.len(),
        ok,
        failed,
        path.display()
    );
    Ok(())
}

