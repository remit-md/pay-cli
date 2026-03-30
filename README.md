# pay CLI

Command-line tool for [pay](https://pay-skill.com) — payment infrastructure for AI agents.

## Install

```bash
cargo install pay-cli
```

## Usage

```bash
pay init                              # First-time setup
pay status                            # Balance + open tabs
pay direct 0xprovider... 5.00         # Send USDC ($1 min)
pay tab open 0xprovider... 20.00 --max-charge 0.50
pay tab list                          # List open tabs
pay request https://api.example.com   # x402 request (auto-pay)
pay webhook register https://hook.me  # Register webhook
pay sign                              # Signer subprocess
```

## License

MIT
