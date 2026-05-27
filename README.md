# Dinopod

Dinopod is a Rust CLI for isolated per-ticket local development environments. It uses Git worktrees, Docker Compose project isolation, and a shared Traefik proxy configured through file-provider routes.

## Install

The intended release path is a prebuilt binary from GitHub Releases with a matching SHA-256 checksum. Contributor installs can use Cargo:

```sh
cargo install --path .
```

Runtime dependencies are intentionally narrow:

- `git`
- `docker`
- `docker compose`
- the pinned Traefik image configured in `dinopod.toml`

## Basic Workflow

Initialize configuration:

```sh
dinopod init
```

Start a ticket environment:

```sh
dinopod dev JIRA-123
```

Expected successful `dev` output includes the worktree path, Compose project name, and local URL. Multiple tickets for the same repo get separate Git worktrees and Docker Compose project names.

Lifecycle commands:

```sh
dinopod list
dinopod list --reconcile
dinopod stop JIRA-123
dinopod down JIRA-123
dinopod down JIRA-123 --volumes
dinopod rm JIRA-123 --yes
```

## Compose Requirements

Dinopod does not edit your Compose file and does not require Traefik labels. It generates a Compose override that attaches the configured app service to the shared proxy network with a unique alias.

Your app service should listen on its normal internal port. Fixed host ports can collide across ticket environments, so Dinopod warns when Compose publishes one.

## Proxy Security

The MVP uses Traefik with the file provider. Traefik does not mount `/var/run/docker.sock` and does not use the Docker provider. Dinopod writes explicit route files that map each ticket hostname to the app container's proxy-network alias.

The default proxy image is `traefik:v3.6`; digest-pinned image references are supported through config.

Concurrent lifecycle commands use a best-effort guard file under the Dinopod config directory. It prevents most accidental overlap but is not a kernel advisory lock.

## Verification

Contributor checks:

```sh
cargo install cargo-deny --version 0.19.7 --locked
cargo fmt --all --check
cargo test --all --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo deny check
```

Release tags build platform archives and `.sha256` checksum files through GitHub Actions. Homebrew and shell installer support are intended follow-ups after the first binary release is stable.

Optional Docker smoke coverage:

```sh
cargo test --test e2e -- --ignored
```
