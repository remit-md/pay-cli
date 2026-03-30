use anyhow::Result;
use clap::Args;

use crate::config::Config;
use crate::error;

#[derive(Args)]
pub struct InitArgs;

pub async fn run(_args: InitArgs, _ctx: super::Context) -> Result<()> {
    if Config::is_initialized() {
        error::success("Already initialized");
        return Ok(());
    }

    let config = Config::default();
    config.save()?;
    error::success("Wallet initialized at ~/.pay/");
    Ok(())
}
