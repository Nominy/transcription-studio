# GitHub Actions

The private GitHub repository is expected to build and package the alpha channel without storing server credentials in git.

## CI

`CI` runs on pushes, pull requests, and manual dispatch:

- Gitleaks scans the full git history with redacted output.
- Windows builds run `cargo fmt --all --check` and `cargo test --workspace --locked` with `RUST_TEST_THREADS=4`. The test step compiles the same workspace graph, so CI skips a separate `cargo check` pass, and the capped libtest worker count avoids over-scheduling the heavy app smoke tests.

## Windows Alpha Package

`Windows Alpha Package` is a manual workflow. It builds the Velopack Windows artifacts and uploads `target/velopack/Releases/*` as a workflow artifact.

Defaults:

- Channel: `alpha`
- Feed URL: `https://reviewgen.ovh/transcription-studio/alpha/win-x64`
- Remote path: `/var/www/transcription-studio/alpha/win-x64`

The workflow downloads the current feed packages before packaging so Velopack can create delta packages when a previous full package exists.

## VPS Publish Secrets

Publishing is disabled by default. To let Actions upload to the VPS, add these repository secrets:

- `ALPHA_SSH_HOST`: VPS host or IP.
- `ALPHA_SSH_USER`: SSH user with write access to the release directory.
- `ALPHA_SSH_PRIVATE_KEY`: private deploy key for that user.
- `ALPHA_SSH_PORT`: optional SSH port, defaults to `22`.

Keep `publish_to_vps` off when you only want a downloadable Actions artifact for local testing.
