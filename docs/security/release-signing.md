# Windows release signing and updater trust

NACC has two independent release signatures. They solve different problems and
both are required for a production release candidate.

1. **Windows Authenticode** signs the NSIS installer/executable with the
   organization's Windows code-signing certificate. Windows uses this to verify
   publisher identity and file integrity.
2. **Tauri updater signing** signs the updater archive with the private key whose
   public verification key is embedded in `src-tauri/tauri.conf.json`. The
   running application rejects updater payloads whose signature does not verify
   against that bundled public key.

The private keys must never be committed to this repository, copied into the
application, printed by CI, or stored on the production VPS.

## GitHub Actions separation

`.github/workflows/desktop-build.yml` is build evidence only. Because
`bundle.createUpdaterArtifacts` is enabled, that workflow generates a throwaway
Tauri updater key so packaging can be exercised. It deletes the resulting
ephemeral updater archive/signature before artifact upload. Its unsigned NSIS
installer is **not distributable**.

`.github/workflows/signed-release-candidate.yml` is the production candidate
path. It is manual, uses the `production-release` GitHub Environment, requires
real signing secrets, verifies the Authenticode result, creates the Tauri
updater archive/signature, creates a static `latest.json`, and uploads the set
as a short-lived Actions artifact. It deliberately does **not** publish a
GitHub Release or modify the live updater channel.

Configure the `production-release` environment with required human reviewers
and these secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- `WINDOWS_CODESIGN_PFX_BASE64`
- `WINDOWS_CODESIGN_PFX_PASSWORD`

The first two must correspond to the updater public key embedded in
`tauri.conf.json`. The last two contain the organization's Authenticode PFX and
password. The workflow imports the PFX only into the ephemeral GitHub-hosted
runner's current-user certificate store and removes it in cleanup.

## Release procedure

1. Bump `src-tauri/tauri.conf.json` to the intended semantic version and commit
   the complete release candidate.
2. Confirm the `production-release` environment has reviewer protection and the
   four secrets above. If the updater private key corresponding to the currently
   bundled public key is unavailable, stop: do not generate a replacement key
   and publish it as though existing clients could trust it. Updater key rotation
   requires an explicit migration plan for already-installed clients.
3. Manually dispatch `signed-release-candidate` on the exact release commit and
   supply a tag such as `v0.1.0`. The workflow refuses a tag whose version does
   not match `tauri.conf.json`.
4. Download the candidate artifact and verify on a clean supported Windows
   machine/VM:
   - installer starts and installs successfully;
   - Windows reports a valid expected publisher signature;
   - application starts with a clean data directory;
   - uninstall/reinstall works without corrupting local state.
5. Exercise the updater against a staging/draft release using the candidate
   `latest.json`, `.nsis.zip`, and `.nsis.zip.sig`. The update must be accepted
   only with the valid signature and must fail when the archive/signature is
   deliberately mismatched.
6. Run the provider smoke/contract checks and the master end-to-end release demo
   required by `docs/RELEASE-CHECKLIST.md`.
7. Only after all release evidence is recorded should a human publish the GitHub
   Release assets (`latest.json`, updater archive/signature, signed installer)
   to the release tag. The configured updater endpoint reads
   `releases/latest/download/latest.json`, so incomplete/partial publication can
   break update checks and must not be exposed as the latest release.

## Key rotation and incident handling

- Treat loss or suspected compromise of either private key as a release
  incident. Stop publishing immediately.
- Authenticode certificate renewal/rotation is independent of the Tauri updater
  key; update the CI PFX secret and verify the new certificate chain before use.
- The Tauri updater public key is compiled into installed clients. Rotating it
  without a compatibility plan strands older clients on the previous trust
  root. Do not change the public key as routine maintenance.
- Never reuse the ephemeral key from `desktop-build.yml` for a real release.

## Evidence required before calling a release production-ready

- GitHub Actions signed-candidate run URL and commit SHA.
- Authenticode `Valid` verification from CI and the clean-machine test.
- Updater success with the correct signed archive and failure with a mismatched
  signature/archive.
- Clean install/start/uninstall evidence.
- Full CI and provider contract/smoke evidence for the exact release commit.
- Human release approval and the final published asset inventory.
