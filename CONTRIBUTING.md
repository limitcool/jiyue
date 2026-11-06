# Contributing to Jiyue

Thank you for looking at Jiyue (霁月). It is an early-stage, local-first Markdown notes app written
in Rust. This file explains how to build, test and submit changes. The user documentation is in
[README.md](README.md) and [docs/](docs/).

## Before you start

- For a large change (a new feature, a change of file formats, a new dependency with a large
  footprint, or anything that touches sync or AI), open an issue first and agree on the scope. Small
  fixes and documentation changes can go straight to a pull request.
- Security problems must not go into public issues. See [SECURITY.md](SECURITY.md).

## Requirements

- Rust: the toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml) (`rustup` installs it
  automatically). The code uses edition 2024.
- The app is built and tested on Windows 11 (the primary platform), and built and tested by CI on
  macOS (Apple silicon and Intel) and Linux. Fonts and native packages differ per platform; see [docs/ci.md](docs/ci.md).
- Linux needs the system packages listed in [docs/ci.md](docs/ci.md#linux-system-packages).

## Build and test

Run these from the repository root. They are the same checks CI runs:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo build --release -p jiyue -p jiyue-mcp --locked
cargo clippy --workspace --all-targets --locked -- -D warnings   # a blocking gate in CI, see docs/ci.md
```

Use `--locked` so that `Cargo.lock` stays the one the project was tested with. If you add or change a
dependency, commit the updated `Cargo.lock` in the same change and say why the dependency is needed.

## Repository layout

| Path | Contents |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | The Cargo workspace (members are in `crates/`). |
| `crates/` | `jiyue-app` (the desktop app, package `jiyue`), `jiyue-mcp` (MCP server), and library crates: core, md, search, sync, ai, export, history, math, diagram, query. |
| `demo-vault/` | A sample vault for trying the app. Copy it before changing anything; tests and helpers must not modify it. |
| `design/` | The final logo and icon set (`design/logo/final/`). |
| `docs/` | User documentation. `docs/dev/` holds contributor notes (the roadmap and the rules for parallel work). |
| `installer/` | The Windows installer (WiX). See [installer/README.md](installer/README.md). |
| `scripts/` | `gen-notices.py`, the third-party notices generator (run by the release jobs), and `run-s3-e2e.ps1`, which runs the S3 end-to-end tests. The macOS and Linux packages are made by cargo-packager, configured in `crates/jiyue-app/Cargo.toml`. |
| `.github/` | CI and release workflows, issue and pull request templates. |

## Pull requests

- Keep a pull request to one topic and describe what changed and how you checked it.
- Add or update tests for behaviour changes. Pure logic should have unit tests.
- Run `cargo fmt --all` before you push. The rustfmt job in CI is a gate.
- Do not commit secrets, personal paths, private hostnames or real notes. Test credentials belong in
  environment variables, not in the repository.
- Documentation in `docs/` and the READMEs must match the behaviour in the change.

## Licence

Jiyue is dual-licensed under MIT or Apache-2.0 (see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE)). Unless you state otherwise, any contribution you submit is licensed
the same way.
