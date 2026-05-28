# Install domain setup (`install.dinopod.dev`)

One-time operator setup to serve the curl installer from GitHub Pages.

## Prerequisites

- Admin access to the `dinogomez/dinopod` GitHub repository
- DNS control for `dinopod.dev`

## GitHub Pages

1. Open **Settings → Pages** for the repository.
2. Under **Build and deployment**, set **Source** to **GitHub Actions**.
3. Merge the deploy workflow (`.github/workflows/deploy-installer.yml`) to `main`.
4. After the first deploy, note the default Pages URL in the workflow run.

## Custom domain

1. Create a DNS **CNAME** record:
   - **Host:** `install`
   - **Target:** `<your-github-pages-host>` (shown in repo Pages settings after first deploy, typically `<user>.github.io`)
2. In **Settings → Pages → Custom domain**, enter `install.dinopod.dev`.
3. Wait for DNS and TLS provisioning (can take up to 24 hours).

The deploy workflow writes `installer-site/CNAME` with `install.dinopod.dev` on each run.

## Verify

After DNS and the first Pages deploy:

```sh
curl -fsSL https://install.dinopod.dev/install.sh | sh
dinopod --version
```

Root URL also works (script is copied to `index.html` for Pages):

```sh
curl -fsSL https://install.dinopod.dev | sh
```

## Smoke checklist (first public release)

1. Tag and push a release (for example `v0.1.0`).
2. Wait for `.github/workflows/release.yml` to finish and publish the release (not draft).
3. Confirm all three Unix artifacts exist on the GitHub Release page with `.sha256` sidecars.
4. Confirm `install.dinopod.dev` serves the current `scripts/install.sh` from `main`.
5. On macOS and Linux test hosts, run the curl installer.
6. Run `dinopod --version` and confirm it matches the release tag.

## Upgrade path

Users upgrade by re-running the same curl command. The installer overwrites the existing binary in `DINOPOD_INSTALL_DIR` (default `~/.local/bin`).

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DINOPOD_VERSION` | latest published release | Pin a specific tag |
| `DINOPOD_INSTALL_DIR` | `~/.local/bin` | Install destination |

## Supported platforms (v1)

- Linux x86_64 (glibc)
- macOS Intel (`x86_64`)
- macOS Apple Silicon (`arm64`)

Windows and Linux ARM64 require manual download from GitHub Releases.
