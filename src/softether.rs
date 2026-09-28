use anyhow::{bail, Context, Result};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

pub struct VpnCmd;

impl VpnCmd {
    pub fn execute(args: &[&str]) -> Result<String> {
        let mut cmd = Command::new("vpncmd.exe");
        cmd.arg("/CLIENT").arg("localhost").arg("/CMD");
        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd
            .output()
            .context("Failed to run vpncmd.exe. Is SoftEther VPN Client installed and vpncmd in PATH?")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let full_output = format!("{}\n{}", stdout, stderr);

        if !output.status.success() {
            bail!(
                "vpncmd command failed with code {:?}: {}",
                output.status.code(),
                full_output.trim()
            );
        }

        Ok(full_output)
    }

    pub fn get_nics() -> Result<Vec<String>> {
        let out = Self::execute(&["NicList"])?;
        let mut nics = Vec::new();
        for line in out.lines() {
            if line.contains("仮想 LAN カード名") || line.contains("Virtual Network Adapter Name") {
                if let Some(val) = line.split('|').nth(1) {
                    let nic = val.trim().to_string();
                    if !nic.is_empty() {
                        nics.push(nic);
                    }
                }
            }
        }
        Ok(nics)
    }

    pub fn ensure_nic(preferred_nic: &str) -> Result<String> {
        let nics = Self::get_nics()?;
        if nics.contains(&preferred_nic.to_string()) {
            return Ok(preferred_nic.to_string());
        }
        if !nics.is_empty() {
            return Ok(nics[0].clone());
        }

        println!("Creating virtual network adapter '{}'...", preferred_nic);
        Self::execute(&["NicCreate", preferred_nic])?;
        Self::execute(&["NicEnable", preferred_nic])?;
        Ok(preferred_nic.to_string())
    }

    pub fn connect(
        account_name: &str,
        ip: &str,
        port: u16,
        hub: &str,
        username: &str,
        nic_name: &str,
    ) -> Result<()> {
        let _ = Self::disconnect(account_name);
        sleep(Duration::from_millis(500));
        let _ = Self::delete_account(account_name);
        sleep(Duration::from_millis(500));

        let server_arg = format!("/SERVER:{}:{}", ip, port);
        let hub_arg = format!("/HUB:{}", hub);
        let user_arg = format!("/USERNAME:{}", username);
        let nic_arg = format!("/NICNAME:{}", nic_name);

        Self::execute(&[
            "AccountCreate",
            account_name,
            &server_arg,
            &hub_arg,
            &user_arg,
            &nic_arg,
        ])
        .context("Failed to create VPN account")?;

        let _ = Self::execute(&["AccountRetrySet", account_name, "/NUM:3", "/RETRYINT:3"]);

        Self::execute(&["AccountConnect", account_name])
            .context("Failed to initiate connection")?;

        Ok(())
    }

    pub fn disconnect(account_name: &str) -> Result<()> {
        let _ = Self::execute(&["AccountDisconnect", account_name]);
        Ok(())
    }

    pub fn delete_account(account_name: &str) -> Result<()> {
        let _ = Self::disconnect(account_name);
        sleep(Duration::from_millis(500));
        let _ = Self::execute(&["AccountDelete", account_name]);
        Ok(())
    }

    pub fn get_status(account_name: &str) -> Result<String> {
        let out = Self::execute(&["AccountStatusGet", account_name])?;
        Ok(out)
    }

    pub fn is_connected(account_name: &str) -> bool {
        if let Ok(out) = Self::get_status(account_name) {
            out.contains("接続完了") || out.contains("Connected")
        } else {
            false
        }
    }

    pub fn list_accounts() -> Result<String> {
        Self::execute(&["AccountList"])
    }
}
