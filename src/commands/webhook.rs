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

    match args.action {
        WebhookAction::Register(a) => {
            let body = serde_json::json!({ "url": a.url });
            let resp = ctx.post("/webhooks", &body).await?;
            if ctx.json {
                error::print_json(&resp);
            } else {
                let id = resp["id"].as_str().unwrap_or("?");
                error::success(&format!("Webhook registered: {id} → {}", a.url));
            }
        }
        WebhookAction::List => {
            let resp = ctx.get("/webhooks").await?;
            if ctx.json {
                error::print_json(&resp);
            } else {
                let hooks = resp.as_array();
                match hooks {
                    Some(hooks) if !hooks.is_empty() => {
                        for wh in hooks {
                            let id = wh["id"].as_str().unwrap_or("?");
                            let url = wh["url"].as_str().unwrap_or("?");
                            error::print_kv(&[("ID", id), ("URL", url)]);
                        }
                    }
                    _ => error::success("No webhooks registered"),
                }
            }
        }
        WebhookAction::Delete(a) => {
            ctx.del(&format!("/webhooks/{}", a.id)).await?;
            if ctx.json {
                error::print_json(&serde_json::json!({ "deleted": a.id }));
            } else {
                error::success(&format!("Webhook deleted: {}", a.id));
            }
        }
    }
    Ok(())
}
