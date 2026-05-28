# Dinopod

Dinopod is a Rust CLI for isolated per-ticket local development environments. It uses Git worktrees, Docker Compose project isolation, and a shared Traefik proxy configured through file-provider routes.

## Install

Install the latest release:

```sh
curl -fsSL https://install.dinopod.dev | sh
```

Supported platforms: Linux x86_64 (glibc), macOS Intel, and macOS Apple Silicon. Pin a version or install directory with environment variables:

```sh
DINOPOD_VERSION=v0.1.0 curl -fsSL https://install.dinopod.dev | sh
DINOPOD_INSTALL_DIR=~/bin curl -fsSL https://install.dinopod.dev | sh
```

Upgrade by re-running the same curl command.

Manual install: download the prebuilt binary for your platform from [GitHub Releases](https://github.com/dinogomez/dinopod/releases) and verify the matching `.sha256` checksum file.

Contributor installs can use Cargo:

```sh
cargo install --path .
```

Operator setup for `install.dinopod.dev` is documented in [docs/install-dns.md](docs/install-dns.md).

Runtime dependencies are intentionally narrow:

- `git`
- `docker`
- `docker compose`
- the pinned Traefik image configured in `dinopod.toml`

## Basic Workflow

Initialize configuration (interactive wizard, or defaults with `-y`):

```sh
dinopod init
dinopod init -y
```

Provision a pod (worktree + isolated Compose + setup commands from `dinopod.toml`):

```sh
dinopod new number-1
```

Run commands in the pod worktree (examples):

```sh
dinopod number-1 pnpm db:migrate
dinopod number-1 pnpm dev:all
```

Lifecycle commands:

```sh
dinopod list
dinopod list --reconcile
dinopod stop number-1
dinopod down number-1
dinopod down number-1 --volumes
dinopod rm number-1 --yes
```

Do not put `docker compose up` in `[setup].commands` — Compose is started by `dinopod new`.

## Compose Requirements

Dinopod does not edit your Compose file and does not require Traefik labels. It generates a Compose override that attaches the configured app service to the shared proxy network with a unique alias.

Your app service should listen on its normal internal port. Fixed host ports can collide across ticket environments, so Dinopod warns when Compose publishes one.

## Proxy Security

The MVP uses Traefik with the file provider. Traefik does not mount `/var/run/docker.sock` and does not use the Docker provider. Dinopod writes explicit route files that map each ticket hostname to the app container's proxy-network alias.

The default proxy image is `traefik:v3.6`; digest-pinned image references are supported through config.

Concurrent mutating lifecycle commands (`new`, `stop`, `down`, `rm`, `list --reconcile`) use a best-effort guard file under the Dinopod config directory. Read-only commands (`list`, `dinopod <id> <command>` passthrough) do not hold the guard, so a long-running dev process in one terminal does not block other shells.

## Verification

Contributor checks:

```sh
cargo install cargo-deny --version 0.19.7 --locked
cargo fmt --all --check
cargo test --all --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo deny check
```

Release tags build platform archives and `.sha256` checksum files through GitHub Actions. Homebrew support is a planned follow-up.

Optional Docker smoke coverage:

```sh
cargo test --test e2e -- --ignored
```
