# In-app updates

Anchor updates itself. On launch it checks GitHub Releases for a newer **signed**
build; if one exists, the dashboard shows an "Update now" banner. One click
downloads the new build (~17 MB — the ~1 GB models and all user data stay put),
verifies its Ed25519 signature against the public key baked into the app,
installs, and relaunches. Silent when there's nothing new or the machine is
offline.

## How it fits together

- **Public key** — `plugins.updater.pubkey` in `tauri.conf.json`. Baked into
  every build; safe to be public.
- **Endpoint** — `plugins.updater.endpoints` points at
  `…/releases/latest/download/latest.json`. That URL resolves to the latest
  **published, non-prerelease** release, so releases must be published as the
  normal "Latest" (see below) — a *pre-release* is invisible to the updater.
- **Signing** — the release workflow builds with `createUpdaterArtifacts` and the
  private updater key (a GitHub secret), producing a signed bundle. It then
  generates `latest.json` (version + signature + download URL) and attaches it,
  plus the installer, to the release.

## Shipping an update (owner)

1. Bump the version in `src-tauri/tauri.conf.json` **and** `src-tauri/Cargo.toml`
   (and `package.json`), e.g. `0.6.1`.
2. Commit, then tag and push:
   ```
   git tag v0.6.1 && git push --tags
   ```
3. CI (`.github/workflows/release.yml`) builds the signed installer + `latest.json`
   and creates a **draft** release. Review it, then click **Publish** (leave it as
   the normal "Latest" — do not mark it pre-release, or the updater won't see it).
4. Every installed Anchor picks it up on its next launch and offers the update.

## The updater signing key (critical)

- The private Ed25519 key is a GitHub Actions secret (`TAURI_SIGNING_PRIVATE_KEY`)
  **and** backed up outside the repo at `%USERPROFILE%\.anchor\anchor-updater.key`.
- It is NOT the paid code-signing certificate — it's the free updater signer.
- **If it is lost, existing installs can never be updated again** (they only trust
  updates signed by the matching key). Keep the backup safe (a password manager).
- `*.key` is gitignored; never commit it.
