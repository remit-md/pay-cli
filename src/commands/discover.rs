use anyhow::{bail, Result};

use crate::error;

#[derive(clap::Args)]
pub struct DiscoverArgs {
    /// Search query (matches keywords and description)
    pub query: Option<String>,

    /// Sort order: volume (default), newest, price_asc, price_desc
    #[arg(long, default_value = "volume")]
    pub sort: String,

    /// Filter by category
    #[arg(long)]
    pub category: Option<String>,

    /// Filter by settlement mode (direct or tab)
    #[arg(long)]
    pub settlement: Option<String>,
}

pub async fn run(args: DiscoverArgs, ctx: super::Context) -> Result<()> {
    let mut params = vec![format!("sort={}", args.sort)];
    if let Some(ref q) = args.query {
        params.push(format!("q={}", q));
    }
    if let Some(ref cat) = args.category {
        params.push(format!("category={}", cat));
    }
    if let Some(ref s) = args.settlement {
        params.push(format!("settlement={}", s));
    }

    let url = format!("{}/discover?{}", ctx.api_url(), params.join("&"));
    let resp = ctx.http.get(&url).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("API error ({status}): {body}");
    }

    let data: serde_json::Value = resp.json().await?;

    if ctx.json {
        error::print_json(&data);
    } else {
        let services = data["services"].as_array();
        match services {
            Some(svcs) if svcs.is_empty() => {
                error::success("No services found.");
            }
            Some(svcs) => {
                // Table header
                println!(
                    "{:<30} {:<25} {:<10} {:<10} {:<10}",
                    "NAME", "DOMAIN", "PRICE", "CALLS/DAY", "SETTLEMENT"
                );
                println!("{}", "-".repeat(85));
                for svc in svcs {
                    let name = svc["name"].as_str().unwrap_or("");
                    let domain = svc["domain"].as_str().unwrap_or("");
                    let daily = svc["daily_calls"].as_i64().unwrap_or(0);
                    let settlement = svc["settlement_mode"].as_str().unwrap_or("");

                    // Extract lowest price from routes
                    let price = svc["routes"]
                        .as_array()
                        .and_then(|routes| routes.iter().filter_map(|r| r["price"].as_str()).next())
                        .unwrap_or("-");

                    println!(
                        "{:<30} {:<25} {:<10} {:<10} {:<10}",
                        truncate(name, 29),
                        truncate(domain, 24),
                        price,
                        daily,
                        settlement,
                    );
                }
            }
            None => {
                error::print_json(&data);
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}
