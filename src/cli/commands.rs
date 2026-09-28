use anyhow::Result;
use clap::Parser;
use std::thread::sleep;
use std::time::Duration;
use tabled::settings::Style;
use tabled::Table;

use super::types::{
    Cli, Commands, ServerRow, DEFAULT_ACCOUNT_NAME, DEFAULT_HUB, DEFAULT_USER,
};
use crate::filter::{check_tcp_reachability, filter_servers, sort_servers, SortKey};
use crate::installer;
use crate::softether::VpnCmd;
use crate::vpngate::{fetch_vpn_servers, VpnServer};

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
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
        }) => {
            if let Some(target_ip) = ip {
                cmd_connect_direct(&target_ip, port, &nic)?;
            } else {
                cmd_auto_connect(
                    sort,
                    country.as_deref(),
                    max_ping,
                    min_speed,
                    &nic,
                    retry_count,
                )?;
            }
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
            if cli.auto {
                cmd_auto_connect(
                    cli.sort,
                    cli.country.as_deref(),
                    cli.max_ping,
                    cli.min_speed,
                    &cli.nic,
                    5,
                )?;
            } else {
                println!("Fetching VPN Gate server list...\n");
                cmd_list(
                    cli.sort,
                    cli.country.as_deref(),
                    cli.max_ping,
                    cli.min_speed,
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

fn cmd_connect_direct(ip: &str, custom_port: Option<u16>, nic_name: &str) -> Result<()> {
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
    VpnCmd::connect(
        DEFAULT_ACCOUNT_NAME,
        ip,
        port,
        DEFAULT_HUB,
        DEFAULT_USER,
        &nic,
    )?;

    wait_for_connection(DEFAULT_ACCOUNT_NAME, ip, port, 10)?;
    Ok(())
}


fn cmd_auto_connect(
    sort: SortKey,
    country: Option<&str>,
    max_ping: Option<u64>,
    min_speed: Option<f64>,
    nic_name: &str,
    retry_count: usize,
) -> Result<()> {
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
        if let Err(e) = VpnCmd::connect(
            DEFAULT_ACCOUNT_NAME,
            &server.ip,
            server.port,
            DEFAULT_HUB,
            DEFAULT_USER,
            &nic,
        ) {
            println!("Failed: {}", e);
            continue;
        }
        println!("Requested.");

        match wait_for_connection(DEFAULT_ACCOUNT_NAME, &server.ip, server.port, 8) {
            Ok(_) => {
                connected = true;
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
    VpnCmd::disconnect(account)?;
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

