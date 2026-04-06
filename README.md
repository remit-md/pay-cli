# pay

Command-line tool for [pay](https://pay-skill.com) — payment infrastructure for AI agents. USDC on Base.

## Install

```bash
cargo install pay-cli
```

## Setup

```bash
pay init          # Create ~/.pay/config.toml
pay status        # Check balance and open tabs
```

## Commands

### Direct Payment

Send a one-shot USDC payment. $1.00 minimum.

```bash
pay direct 0xprovider... 5.00
pay direct 0xprovider... 1.50 --memo "task-42"
```

### Tab Management

Open, charge, top up, and close pre-funded metered tabs. $5.00 minimum to open.

```bash
pay tab open 0xprovider... 20.00 --max-charge 0.50
pay tab list
pay tab topup tab_abc123 10.00
pay tab close tab_abc123
```

Provider-side charging:

```bash
pay tab charge tab_abc123 0.30
```

### x402 Requests

Make HTTP requests that automatically handle 402 Payment Required responses.

```bash
pay request https://api.example.com/data
```

The CLI detects 402 responses, pays via direct or tab settlement, and retries.

### Webhooks

```bash
pay webhook register https://myapp.com/hooks
pay webhook list
pay webhook delete wh_abc123
```

### Signer

The `pay sign` command acts as a signing subprocess for SDKs. It reads a hex-encoded hash from stdin and writes the signature to stdout.

```bash
echo "deadbeef..." | pay sign
```

Private key is loaded from the `PAYSKILL_SIGNER_KEY` environment variable.

### Funding

```bash
pay fund                              # Open Coinbase Onramp funding page
pay withdraw 0xrecipient... 50.00     # Get withdrawal link
```

## Global Flags

| Flag | Env Var | Purpose |
|------|---------|---------|
| `--json` | — | Output JSON instead of human-readable format |
| `--api-url` | `PAYSKILL_API_URL` | Override API URL (default: `https://pay-skill.com/api/v1`) |

## Configuration

Config file: `~/.pay/config.toml`

```toml
api_url = "https://pay-skill.com/api/v1"
testnet = false
```

| Env Var | Purpose |
|---------|---------|
| `PAYSKILL_API_URL` | Override API URL |
| `PAYSKILL_SIGNER_KEY` | Private key for `pay sign` |

## Command Reference

```
pay init                              First-time setup
pay status                            Balance + open tabs
pay direct <to> <amount>              Send USDC ($1 min)
pay tab open <provider> <amount>      Open tab ($5 min)
  --max-charge <amount>               Max per-charge limit
pay tab charge <tab_id> <amount>      Charge a tab (provider-side)
pay tab close <tab_id>                Close a tab
pay tab topup <tab_id> <amount>       Add funds to open tab
pay tab list                          List open tabs
pay request <url>                     x402 request (auto-pay)
  -X <METHOD>                          HTTP method (default: GET, POST if -d)
  -H "Key: Value"                      Add header (repeatable)
  -d <body>                            Request body (@file reads from file)
  -o <file>                            Write response to file
  -v / -s                              Verbose / silent
  --no-pay                             Skip x402 payment handling
pay webhook register <url>            Register webhook endpoint
pay webhook list                      List registered webhooks
pay webhook delete <id>               Remove a webhook
pay sign                              Signer subprocess (stdin/stdout)
pay fund                              Open funding page
pay withdraw <to> <amount>            Withdraw USDC
```

## License

MIT
