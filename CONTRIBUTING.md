# Contributing

Thanks for your interest in improving `media-maintenance`.

## Development checks

Before opening a pull request, run:

```bash
cargo fmt --check
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

## Project principles

- Prefer dry-run/report-first behaviour for risky operations.
- Keep scheduled jobs short-lived and deterministic.
- Avoid background daemons or long-running processes.
- Do not add secret values to examples, tests, docs, or reports.
- Keep write actions explicit and easy to audit.

## Pull requests

A good pull request should include:

- a short description of the operational problem;
- the safety behaviour for dry-run vs apply mode;
- example report output when relevant;
- any migration notes for existing deployments.
