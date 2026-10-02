mod cli;
mod filter;
mod installer;
mod softether;
mod split;
mod vpngate;

fn main() -> anyhow::Result<()> {
    cli::run()
}

