# SoftEther CLI

`secli` - Connect VPN Gate from command line, using SoftEther VPN Client. For agents or any softwares.

## Features

- **VPN Gate API**: Fetch available VPN servers via official VPN Gate API.
- **Auto Connect**:
  - `Ping (Low to High)`: Sort: response time
  - `Country (A-Z)`: Sort: server position
  - `Speed (High to Low)`: Sort: connection speed
  - `Score`: Sort: score
  - `Sessions`: Sort: connected sessions
- **Safe Auto Connect**:
  - Check for TCP Reachability to select server to connect, skipping unavailable / timed out servers.
- **Easy Setup**:
  - `secli install` to install quickly including PATH registeration.
- **Lifecycle Management**:
  - Auto detect / Create / Enable Virtual NIC.
  - Create / Connect / Check status / Disconnect / Delete connected accounts.
- **Connect to Specific Domains**
  - `--focus`: Only listed domains / IPs will be connected via VPN, everything else stays original connections..
  - `--exclude`: Listed domains / IPs will stay on the original connection, existing connections won't be disconnected.

---

## Installation

```powershell
# Build by yourself (or get installer from Releases)
cargo build --release

# Run installer
.\SoftEther-CLI.exe install
```

---

## Usage

### 1. List available VPN Gate servers (`list`)

```powershell
# Default (Sort by ping, 25 servers)
secli list

# Servers on Japan, sort by ping, 10 servers
secli list -c JP -s ping -n 10

# Sort by connecction speed, 15 servers
secli list -s speed -n 15

# Sort by country (A-Z)
secli list -s country

# Only under 50 ms, over 50 Mbps
secli list --max-ping 50 --min-speed 50
```

### 2. Auto connect (`connect` / `--auto`)

Connect the best matching server automatically.

```powershell
# Lowest ping
secli connect

# Server on Japan, sort by lowest ping
secli connect -c JP -s ping

# Sort by connection speed
secli connect -s speed
```

### 3. Direct connect for specific IP

```powershell
# Automatically determines port
secli connect 219.100.37.7

# Add express port
secli connect 219.100.37.7 -p 443
```

### 4. Specify domains

- `--focus` allows you to connect for only specified domains / IPs.
- `--exclude` allows you to keep connections to all IPs except specified domains / IPs on the original route.

Run as Administrator is required on Windows.

```powershell
# Only Discord goes via VPN, everything else stays direct
secli connect --focus https://discord.com

# Hypixel stays on the original connection and won't be disconnected.
secli connect --exclude https://hypixel.net

# Multiple targets or IP/CIDR specification is supported.
secli connect --focus discord.com, fes.discord.com --focus 1.1.1.1

# Connect with global arguments also work
secli connect --exclude hypixel.net --exclude 20.0.0.0/8 -a

# DNS changes? Re-resolve and refresh host routes
secli split refresh

# Keep following DNS (Ex: Hypixel / Discord rotate IPs)
secli split watch --interval 300

# Inspect / remove split routes
secli split status
secli split clear / secli disconnect
```

> [!NOTE]
> You cannot combine --focus and --exclude at the same time

### 5. Check current connection (`status`)

View current connection status (Session info / Server info / Encryption method / Network usage).

```powershell
secli status
```

### 6. Disconnect (`disconnect`)

Disconnect from VPN server.

```powershell
secli disconnect
```

### 7. Check virtual LAN Card, accounts

```powershell
# List virtual LAN card (NIC)
secli nics

# Accounts in SoftEther VPN Client
secli accounts
```

---

## Commands / Args

```text
Commands:
  install     Install to current user, register PATH
  list        List available VPN Gate servers
  connect     Connect the best matching server automatically
  split       Inspect / refresh / clear split-tunnel host routes
  disconnect  Disconnect from VPN server (Also clears split routes)
  status      View current connection status
  accounts    Check accounts
  nics        Check virtual LAN Card
  help        Show help

Global / Connect / List Options:
  -a, --auto                   Auto connect
  -s, --sort <SORT>            Sort: [ping, country, speed, score, sessions] (Default: ping)
  -c, --country <COUNTRY>      Country code (ISO 3166-1 Alpha-2, Ex: JP, US)
      --max-ping <MAX_PING>    Maximum response time (ms)
      --min-speed <MIN_SPEED>  Minimum connection speed (Mbps)
      --nic <NIC>              Virtual LAN card to use (Default: VPN)
      --focus <DOMAIN|IP|CIDR> Only these via VPN (repeatable, comma-separated OK)
      --exclude <DOMAIN|IP|CIDR> Keep these on original connection, TCP preserved

Split subcommands:
  split status                 Show saved split targets / routes
  split refresh                Re-resolve domains and refresh host routes
  split watch --interval <SEC> Keep refreshing (default 300s, min 10s)
  split clear                  Remove split host routes
```
