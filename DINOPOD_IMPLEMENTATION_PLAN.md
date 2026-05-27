# Dinopod Implementation Plan

## One-line goal

Build `dinopod`, a Rust CLI that creates isolated, collision-free local development environments per ticket/branch using Git worktrees, Docker Compose project isolation, and a shared Dockerized reverse proxy.

Example target UX:

```bash
dinopod dev JIRA-123
```

Expected output:

```text
Created or reused worktree: ../.dinopod-worktrees/myapp-jira-123
Started Docker Compose project: myapp-jira-123
URL: http://jira-123.localhost
```

---

## Problem statement

When working on multiple tickets at once, a developer often needs multiple copies of the same app running at the same time.

Each ticket should have:

- Its own Git worktree or working directory.
- Its own Docker Compose project name.
- Its own app container.
- Its own database container and persistent database volume.
- Its own dev URL.
- No host port collisions.
- No manual editing of `.env`, ports, or Compose files per ticket.

Desired shape:

```text
ticket-1.localhost -> ticket 1 app container -> ticket 1 db
ticket-2.localhost -> ticket 2 app container -> ticket 2 db
ticket-3.localhost -> ticket 3 app container -> ticket 3 db
```

The app can keep listening on the same internal port, such as `3000`, in every environment. Routing should happen by hostname through one shared reverse proxy.

---

## Hard dependencies

The developer machine should only need these installed:

```text
git
docker
docker compose
dinopod
```

Everything else should run inside Docker.

### Important dependency decision

Do not require users to install Traefik, nginx, Caddy, mkcert, Kubernetes, DevPod, DDEV, Lando, or Portless for the MVP.

Use Traefik as a Docker container managed by `dinopod`.

Portless can be evaluated later as an optional backend, especially for non-Docker processes, HTTPS-first workflows, or LAN sharing. It is not necessary for the MVP because Docker + Traefik already solves hostname routing without host port collisions.

---

## MVP architecture

```text
User repo
  ├── docker-compose.yml
  ├── dinopod.toml
  └── app source

dinopod
  ├── creates/reuses git worktree per ticket
  ├── starts shared reverse proxy if missing
  ├── runs docker compose with unique COMPOSE_PROJECT_NAME
  ├── injects APP_HOST and DINOPOD_* env values
  └── prints final URL

Docker
  ├── shared network: dinopod-proxy
  ├── shared reverse proxy: dinopod-traefik
  └── isolated Compose project per ticket
      ├── app container
      ├── db container
      ├── project-scoped network
      └── project-scoped volumes
```

---

## Routing model

Use a shared Traefik container connected to an external Docker network:

```text
dinopod-proxy
```

Each app container joins:

```text
default Compose project network
dinopod-proxy
```

Each app container gets Traefik labels that map a hostname to its internal app port.

Example:

```text
Host(`jira-123.localhost`) -> app container port 3000
```

Do not publish each app's port to the host. Use `expose`, not `ports`, for the app service.

---

## Naming model

Given:

```text
repo name: myapp
ticket: JIRA-123
```

Normalize to:

```text
ticket slug: jira-123
project name: myapp-jira-123
host: jira-123.localhost
worktree path: ../.dinopod-worktrees/myapp-jira-123
```

Rules:

- Lowercase.
- Replace invalid hostname/project-name characters with `-`.
- Collapse repeated `-`.
- Trim leading/trailing `-`.
- Keep names deterministic.
- Allow overrides in config or CLI flags.

---

## Target commands

### `dinopod init`

Create a starter `dinopod.toml` in the current repo.

```bash
dinopod init --service app --port 3000
```

Should create:

```toml
[app]
service = "app"
internal_port = 3000
compose_file = "docker-compose.yml"
default_branch = "main"

[worktree]
root = "../.dinopod-worktrees"

[proxy]
host_suffix = "localhost"
network = "dinopod-proxy"
container_name = "dinopod-traefik"
http_port = 80
```

### `dinopod dev <ticket>`

Create or reuse a worktree for the ticket, start the proxy if needed, run Docker Compose, and print the URL.

```bash
dinopod dev JIRA-123
```

Internally:

```bash
git worktree add ../.dinopod-worktrees/myapp-jira-123 -b JIRA-123
COMPOSE_PROJECT_NAME=myapp-jira-123 APP_HOST=jira-123.localhost docker compose up -d
```

The actual implementation should handle cases where:

- The branch already exists.
- The worktree already exists.
- The compose project is already running.
- The user is currently inside a worktree.
- The current repo has uncommitted work.
- Docker is not running.
- Port 80 is already taken.

### `dinopod list`

List active Dinopod environments.

```bash
dinopod list
```

Example output:

```text
PROJECT             TICKET     URL                         STATUS
myapp-jira-123      JIRA-123   http://jira-123.localhost   running
myapp-jira-456      JIRA-456   http://jira-456.localhost   running
```

Initial implementation may derive this from Docker Compose labels and/or a local state file.

### `dinopod stop <ticket>`

Stop the Compose project but keep the worktree and volumes.

```bash
dinopod stop JIRA-123
```

Equivalent behavior:

```bash
COMPOSE_PROJECT_NAME=myapp-jira-123 docker compose stop
```

### `dinopod down <ticket>`

Stop and remove containers/networks, but keep DB volumes unless `--volumes` is passed.

```bash
dinopod down JIRA-123
dinopod down JIRA-123 --volumes
```

### `dinopod rm <ticket>`

Remove the environment.

Suggested behavior:

```bash
dinopod down JIRA-123
git worktree remove ../.dinopod-worktrees/myapp-jira-123
```

Ask for confirmation unless `--yes` is passed.

### `dinopod proxy start`

Start or repair the shared proxy.

```bash
dinopod proxy start
```

### `dinopod proxy stop`

Stop the shared proxy.

```bash
dinopod proxy stop
```

---

## Suggested Rust crates

Use a minimal Rust stack:

```toml
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
directories = "5"
```

Optional later:

```toml
duct = "0.13"       # nicer command execution
owo-colors = "4"    # CLI colors
tabled = "0.16"     # table output for list
which = "6"         # dependency checks
```

MVP can use `std::process::Command` directly.

---

## Files to implement

```text
src/main.rs
src/cli.rs
src/config.rs
src/git.rs
src/docker.rs
src/proxy.rs
src/names.rs
src/state.rs
```

A single-file MVP is acceptable, but the above modules are the desired structure once stabilized.

---

## Config file

Create and read `dinopod.toml`.

Example:

```toml
[app]
service = "app"
internal_port = 3000
compose_file = "docker-compose.yml"
default_branch = "main"

[worktree]
root = "../.dinopod-worktrees"

[proxy]
host_suffix = "localhost"
network = "dinopod-proxy"
container_name = "dinopod-traefik"
http_port = 80
```

Config resolution:

1. CLI flags override config.
2. `dinopod.toml` overrides defaults.
3. Defaults are used if config values are missing.

---

## Required Compose conventions

The app's `docker-compose.yml` should not publish a fixed host port for the web app.

Prefer this:

```yaml
services:
  app:
    build: .
    expose:
      - "3000"
    environment:
      DATABASE_URL: postgres://postgres:postgres@db:5432/app
    labels:
      - traefik.enable=true
      - traefik.http.routers.${COMPOSE_PROJECT_NAME}.rule=Host(`${APP_HOST}`)
      - traefik.http.services.${COMPOSE_PROJECT_NAME}.loadbalancer.server.port=3000
      - traefik.docker.network=dinopod-proxy
    networks:
      - default
      - dinopod-proxy

  db:
    image: postgres:16
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: app
    volumes:
      - db:/var/lib/postgresql/data

volumes:
  db:

networks:
  dinopod-proxy:
    external: true
```

Note:

- `${COMPOSE_PROJECT_NAME}` must be unique per ticket.
- `${APP_HOST}` must be unique per ticket.
- Named volume `db` becomes project-scoped by Docker Compose, so each ticket gets a separate DB volume.

---

## Shared proxy Compose file

`dinopod` can generate a temporary Compose file or run this directly.

```yaml
services:
  traefik:
    image: traefik:v3
    container_name: dinopod-traefik
    command:
      - --providers.docker=true
      - --providers.docker.exposedbydefault=false
      - --entrypoints.web.address=:80
    ports:
      - "80:80"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    networks:
      - dinopod-proxy

networks:
  dinopod-proxy:
    external: true
```

Before starting proxy:

```bash
docker network create dinopod-proxy
```

Ignore the error if the network already exists.

---

## Environment variables injected by `dinopod dev`

```text
COMPOSE_PROJECT_NAME=myapp-jira-123
APP_HOST=jira-123.localhost
DINOPOD_TICKET=JIRA-123
DINOPOD_PROJECT=myapp-jira-123
DINOPOD_URL=http://jira-123.localhost
```

Optional future values:

```text
DINOPOD_WORKTREE_PATH=../.dinopod-worktrees/myapp-jira-123
DINOPOD_ROOT_REPO=/absolute/path/to/repo
```

---

## Worktree behavior

`dinopod dev JIRA-123` should:

1. Confirm the current directory is inside a Git repo.
2. Determine the repo root.
3. Determine repo name from the root folder unless overridden.
4. Normalize ticket into a slug.
5. Compute the worktree path.
6. If worktree exists, reuse it.
7. If branch exists, use it.
8. If branch does not exist, create it from the configured default branch.
9. Run Docker Compose from inside the worktree path.

Suggested Git commands:

```bash
git rev-parse --show-toplevel
git worktree list --porcelain
git show-ref --verify --quiet refs/heads/JIRA-123
git worktree add <path> <branch>
git worktree add -b <branch> <path> <base>
```

Branch strategy:

- Default: branch name equals ticket input exactly, e.g. `JIRA-123`.
- Add `--branch <name>` override.
- Add `--base <branch>` override.

---

## State tracking

MVP can avoid state files and derive from Docker + Git.

However, a local state file is useful:

```text
~/.config/dinopod/state.toml
```

Example:

```toml
[[env]]
repo = "myapp"
ticket = "JIRA-123"
project = "myapp-jira-123"
host = "jira-123.localhost"
url = "http://jira-123.localhost"
worktree = "/Users/dino/code/.dinopod-worktrees/myapp-jira-123"
created_at = "2026-05-28T00:00:00Z"
```

If using state, never trust it blindly. Docker and Git are source of truth.

---

## Error handling expectations

The CLI should give clear errors for:

- `git` missing.
- `docker` missing.
- `docker compose` missing.
- Not inside a Git repo.
- Docker daemon not running.
- Port `80` already in use.
- Compose file not found.
- App service missing from Compose file.
- Proxy network missing and cannot be created.
- Worktree path exists but is not the expected Git worktree.
- Ticket slug becomes empty after normalization.

Suggested error style:

```text
Error: Docker is not running.

Try:
  open Docker Desktop
  or run: sudo systemctl start docker
```

---

## MVP acceptance criteria

A developer can run:

```bash
dinopod init --service app --port 3000
dinopod dev JIRA-123
dinopod dev JIRA-456
```

And both environments are accessible:

```text
http://jira-123.localhost
http://jira-456.localhost
```

Both apps may use internal port `3000`.

There should be no host port collision.

Each environment should have a separate database volume.

This should work without manually editing ports or `.env` values.

---

## Test scenarios

### Scenario 1: two tickets at once

```bash
dinopod dev JIRA-123
dinopod dev JIRA-456
```

Expected:

- Two worktrees.
- Two Compose projects.
- Two app containers.
- Two DB containers.
- Two DB volumes.
- Two URLs.
- No port conflict.

### Scenario 2: rerun same ticket

```bash
dinopod dev JIRA-123
dinopod dev JIRA-123
```

Expected:

- Reuses existing worktree.
- Reuses or restarts Compose project.
- Prints the same URL.
- Does not fail due to existing branch/worktree.

### Scenario 3: stop without deleting data

```bash
dinopod stop JIRA-123
dinopod dev JIRA-123
```

Expected:

- App restarts.
- DB volume remains.

### Scenario 4: remove with volumes

```bash
dinopod down JIRA-123 --volumes
```

Expected:

- Containers removed.
- Project network removed.
- DB volume removed.

### Scenario 5: app has fixed host port

If the user's Compose file has:

```yaml
ports:
  - "3000:3000"
```

Expected:

- Warn that fixed host ports can cause collisions.
- Suggest using `expose: ["3000"]` instead.
- Do not silently edit the file unless a future `dinopod doctor --fix` command is implemented.

---

## Future enhancements

### `dinopod doctor`

Validate:

- Docker is running.
- Compose is available.
- Git repo is valid.
- `dinopod.toml` is valid.
- Compose file exists.
- App service exists.
- App service has Traefik labels.
- App service does not publish fixed host ports.
- Proxy is running.
- Proxy network exists.

### `dinopod open <ticket>`

Open the URL in the browser.

```bash
dinopod open JIRA-123
```

### `dinopod logs <ticket>`

Follow logs for a ticket environment.

```bash
dinopod logs JIRA-123
dinopod logs JIRA-123 app
```

### HTTPS support

Options:

1. Stay HTTP-only for MVP.
2. Add local TLS later.
3. Evaluate Portless as an optional backend.
4. Evaluate Traefik TLS with generated certificates.

### Portless backend

Possible future config:

```toml
[proxy]
backend = "traefik" # or "portless"
```

Only implement after the Docker + Traefik MVP is working.

### Remote/team preview environments

Out of scope for MVP.

Could later support:

- Kubernetes.
- Fly.io.
- Railway.
- Render.
- Cloudflare Tunnel.
- GitHub PR comments.
- CI-created preview environments.

---

## Non-goals for MVP

Do not build these yet:

- Kubernetes support.
- Cloud preview environments.
- Team sharing.
- Authentication.
- HTTPS automation.
- Database cloning from production.
- Secrets manager integration.
- Full Compose parser/rewriter.
- GUI/TUI.
- Devcontainer orchestration.

---

## Suggested implementation order

1. Create Rust CLI with `clap`.
2. Implement config loading from `dinopod.toml`.
3. Implement name normalization.
4. Implement dependency checks.
5. Implement `dinopod init`.
6. Implement Git repo detection.
7. Implement worktree creation/reuse.
8. Implement proxy network creation.
9. Implement proxy startup.
10. Implement `docker compose up -d` with env injection.
11. Print URL.
12. Implement `list`.
13. Implement `stop`.
14. Implement `down`.
15. Implement `rm`.
16. Add tests for name normalization and config parsing.
17. Add README examples.

---

## Language and distribution decision

Implement `dinopod` as a Rust CLI.

Reasoning:

- The CLI mostly orchestrates `git`, `docker`, and `docker compose`.
- Rust gives us a fast, reliable, single-binary tool.
- Users should not need Node.js, Python, or a language runtime just to use `dinopod`.
- Adoption can still be easy if we ship prebuilt binaries and package-manager installs.
- Do not make `cargo install` the primary installation path for normal users.

The key adoption rule:

```text
Rust internally, polished installer externally.
```

Normal users should install with:

```bash
brew install dinopod
```

or:

```bash
curl -fsSL https://install.dinopod.dev | sh
```

Then use:

```bash
dinopod dev JIRA-123
```

They should not need to know Rust exists.

---

## Installation and release strategy

### Required machine dependencies for users

Users should only need:

```text
dinopod
git
docker
docker compose
```

Users should not need:

```text
rust
cargo
node
npm
python
traefik installed locally
portless
kubernetes
```

Traefik is allowed only as a Docker container started by `dinopod`.

### Primary install methods

Support these as first-class install paths:

```bash
brew install dinopod
```

```bash
curl -fsSL https://install.dinopod.dev | sh
```

These should download a prebuilt binary for the user's OS and CPU architecture.

### Secondary install methods

Also support:

```bash
cargo install dinopod
```

```bash
npm install -g dinopod
```

```bash
npx dinopod dev JIRA-123
```

Notes:

- `cargo install` is for Rust users and contributors.
- The npm package should be an optional wrapper that downloads and executes the Rust binary.
- The npm package should not be the main implementation.
- The Rust binary remains the source of truth.

### GitHub Releases artifacts

Each release should publish binaries like:

```text
dinopod-aarch64-apple-darwin.tar.gz
dinopod-x86_64-apple-darwin.tar.gz
dinopod-x86_64-unknown-linux-gnu.tar.gz
dinopod-aarch64-unknown-linux-gnu.tar.gz
dinopod-x86_64-pc-windows-msvc.zip
```

Later we can add:

```text
dinopod-aarch64-pc-windows-msvc.zip
dinopod-x86_64-unknown-linux-musl.tar.gz
dinopod-aarch64-unknown-linux-musl.tar.gz
```

### Release tooling

Use `cargo-dist` or a similar Rust release tool to automate:

- Cross-platform builds.
- GitHub Releases.
- Checksums.
- Shell installer script.
- Homebrew formula/tap updates.
- Optional npm wrapper publishing later.

Preferred initial release path:

```bash
cargo dist init
git tag v0.1.0
git push origin v0.1.0
```

The exact commands may vary after `cargo-dist` configuration, but the repository should be designed around automated tagged releases.

### Homebrew

Create a Homebrew tap once the first release exists.

Target install UX:

```bash
brew install dinopod
```

Or, if using a tap initially:

```bash
brew tap dinopod/tap
brew install dinopod
```

### Shell installer

Provide an installer script that:

1. Detects OS.
2. Detects architecture.
3. Downloads the matching GitHub Release artifact.
4. Verifies checksum if available.
5. Installs the binary to a user-writable path, such as `~/.local/bin`.
6. Prints PATH instructions if needed.

Target UX:

```bash
curl -fsSL https://install.dinopod.dev | sh
```

### npm wrapper

The npm package is optional but useful for web developers.

Target UX:

```bash
npm install -g dinopod
npx dinopod dev JIRA-123
pnpm dlx dinopod dev JIRA-123
bunx dinopod dev JIRA-123
```

Implementation idea:

- Publish a small npm package named `dinopod`.
- During install or first run, download the correct Rust binary.
- Cache the binary.
- Execute it with forwarded args.
- Keep the Rust binary as the canonical implementation.

This lets web developers try the CLI through familiar Node tooling without making TypeScript the core implementation language.

---

## Codex implementation instruction

Please implement this as a Rust CLI in the current repository.

Prioritize a working MVP over abstractions.

Use only these machine-level dependencies:

```text
git
docker
docker compose
```

Use Traefik only as a Docker container started by the CLI.

Do not require Portless for the MVP.

Do not require Node.js, npm, Python, Kubernetes, DDEV, Lando, or DevPod for the MVP.

The first version should be installable for developers through prebuilt binaries. Include repository structure and release configuration that can later support:

```text
GitHub Releases
Homebrew
shell installer
crates.io
optional npm wrapper
```

The most important success path is:

```bash
dinopod init --service app --port 3000
dinopod dev JIRA-123
dinopod dev JIRA-456
```

Both URLs should work at the same time without changing app ports:

```text
http://jira-123.localhost
http://jira-456.localhost
```
