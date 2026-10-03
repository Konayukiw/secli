mod cli;
mod filter;
mod installer;
mod softether;
mod route;
mod vpngate;

fn main() -> anyhow::Result<()> {
    cli::run()
}

