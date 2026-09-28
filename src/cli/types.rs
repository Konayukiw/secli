use clap::{Parser, Subcommand};
use crate::filter::SortKey;
use crate::vpngate::VpnServer;
use tabled::Tabled;

pub const DEFAULT_ACCOUNT_NAME: &str = "vpngate_cli";
pub const DEFAULT_NIC_NAME: &str = "VPN";
pub const DEFAULT_HUB: &str = "VPNGATE";
pub const DEFAULT_USER: &str = "vpn";

#[derive(Parser)]
#[command(
    name = "secli",
    about = "SoftEther VPN Gate CLI - Connect to VPN Gate servers directly via SoftEther VPN Client",
    version = "0.1.0"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(long, short = 'a')]
    pub auto: bool,

    #[arg(long, short = 's', value_enum, default_value = "ping")]
    pub sort: SortKey,

    #[arg(long, short = 'c')]
    pub country: Option<String>,

    #[arg(long)]
    pub max_ping: Option<u64>,

    #[arg(long)]
    pub min_speed: Option<f64>,

    #[arg(long, default_value = DEFAULT_NIC_NAME)]
    pub nic: String,
}

#[derive(Subcommand)]
pub enum Commands {
    Install,
    List {
        #[arg(long, short = 's', value_enum, default_value = "ping")]
        sort: SortKey,

        #[arg(long, short = 'c')]
        country: Option<String>,

        #[arg(long)]
        max_ping: Option<u64>,

        #[arg(long)]
        min_speed: Option<f64>,

        #[arg(long, short = 'n', default_value = "25")]
        limit: usize,
    },

    Connect {
        #[arg(value_name = "IP")]
        ip: Option<String>,

        #[arg(long, short = 'p')]
        port: Option<u16>,

        #[arg(long, default_value = DEFAULT_NIC_NAME)]
        nic: String,

        #[arg(long, short = 's', value_enum, default_value = "ping")]
        sort: SortKey,

        #[arg(long, short = 'c')]
        country: Option<String>,

        #[arg(long)]
        max_ping: Option<u64>,

        #[arg(long)]
        min_speed: Option<f64>,

        #[arg(long, default_value = "5")]
        retry_count: usize,
    },

    Disconnect {
        #[arg(default_value = DEFAULT_ACCOUNT_NAME)]
        account: String,
    },

    Status {
        #[arg(default_value = DEFAULT_ACCOUNT_NAME)]
        account: String,
    },

    Accounts,
    Nics,
}

#[derive(Tabled)]
pub struct ServerRow {
    #[tabled(rename = "Country")]
    pub country: String,
    #[tabled(rename = "IP Address")]
    pub ip: String,
    #[tabled(rename = "Port")]
    pub port: u16,
    #[tabled(rename = "Ping (ms)")]
    pub ping: String,
    #[tabled(rename = "Speed (Mbps)")]
    pub speed: String,
    #[tabled(rename = "Sessions")]
    pub sessions: u64,
    #[tabled(rename = "HostName")]
    pub hostname: String,
}

impl From<&VpnServer> for ServerRow {
    fn from(s: &VpnServer) -> Self {
        ServerRow {
            country: format!("{} ({})", s.country_short, s.country_long),
            ip: s.ip.clone(),
            port: s.port,
            ping: s
                .ping
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string()),
            speed: format!("{:.2}", s.speed_mbps()),
            sessions: s.num_vpn_sessions,
            hostname: s.hostname.clone(),
        }
    }
}
