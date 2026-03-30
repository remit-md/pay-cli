use anyhow::Result;
use clap::{Args, Subcommand};

use crate::error;

#[derive(Args)]
pub struct WebhookArgs {
    #[command(subcommand)]
    pub action: WebhookAction,
}

#[derive(Subcommand)]
pub enum WebhookAction {
    /// Register a webhook endpoint
    Register(WebhookRegisterArgs),
    /// List registered webhooks
    List,
    /// Delete a webhook
    Delete(WebhookDeleteArgs),
}

#[derive(Args)]
pub struct WebhookRegisterArgs {
    /// Webhook URL
    pub url: String,
}

#[derive(Args)]
pub struct WebhookDeleteArgs {
    /// Webhook ID
    pub id: String,
}

pub async fn run(args: WebhookArgs, ctx: super::Context) -> Result<()> {
    super::require_init()?;
    let _ = ctx.api_url();

    match args.action {
        WebhookAction::Register(a) => {
            if ctx.json {
                error::print_json(&serde_json::json!({
                    "status": "not_implemented",
                    "url": a.url,
                }));
            } else {
                error::success(&format!(
                    "Webhook registered: {} (not yet connected)",
                    a.url
                ));
            }
        }
        WebhookAction::List => {
            if ctx.json {
                error::print_json(&serde_json::json!({
                    "status": "not_implemented",
                    "webhooks": [],
                }));
            } else {
                error::success("No webhooks (not yet connected to server)");
            }
        }
        WebhookAction::Delete(a) => {
            if ctx.json {
                error::print_json(&serde_json::json!({
                    "status": "not_implemented",
                    "deleted": a.id,
                }));
            } else {
                error::success(&format!("Webhook deleted: {} (not yet connected)", a.id));
            }
        }
    }
    Ok(())
}
