# pay-cli

`pay` command-line tool. Rust binary.

## Reference
- Remit CLI: `C:\Users\jj\remit-cli\` (frozen, reference only)
- Dev guide: `C:\Users\jj\payskill\spec\guides\CLI.md`
- General guide: `C:\Users\jj\payskill\spec\guides\GENERAL.md`
- Project spec: `C:\Users\jj\payskill\spec\CLAUDE.md`

## Quick Rules
- Rust stable, clap for arg parsing
- **No `.unwrap()` or `.expect()`.** All commands return `Result<(), AppError>`.
- No panics. Every error path returns a Result.
- No interactive prompts except `pay init`
- No hardcoded URLs — config file or `--api-url` flag
- Signer: OS keychain default, .enc fallback, `PAYSKILL_SIGNER_KEY` env var
- `pay sign` subprocess: stdin → hash, stdout → signature
- Exit codes: 0 success, 1 user error, 2 system error
- Distribution: Homebrew, Scoop, npm, PyPI, crates.io, GitHub Releases
