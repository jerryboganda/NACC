# NACC public-distribution release checklist (future scope)

This checklist is retained for a possible future decision to distribute NACC.
It is **not** part of the current single-user local-only readiness contract and
unchecked items here must not downgrade or block personal local use.

Every line states its evidence requirement. Items marked **[CI/signed]** need
the protected `production-release` GitHub Environment, real signing material,
and a manual CI dispatch. The repository now contains the signed-candidate
workflow and the signing procedure, but this working tree has not dispatched it
and therefore cannot claim that release evidence. Do not tick an item without
the exact run/test evidence.

## Build
- [ ] `cargo +1.96.0-x86_64-pc-windows-gnu clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo +1.96.0-x86_64-pc-windows-gnu test --workspace`
- [ ] `npm run build && npm test`
- [ ] `cargo +1.96.0-x86_64-pc-windows-gnu run -p nacc-app -- --export-bindings` (bindings fresh)
- [ ] **[CI/signed]** `signed-release-candidate.yml` builds the MSVC NSIS installer for the exact release commit
- [ ] **[CI/signed]** Authenticode verification is `Valid` for every produced installer
- [ ] **[CI/signed]** Tauri updater archive + `.sig` + `latest.json` produced by the protected workflow

## Docs shipped in-repo (this build)
- [x] docs/security/threat-model.md
- [x] docs/security/release-signing.md
- [x] docs/architecture/adr-0001-adapter-contract-deferrals.md
- [x] docs/architecture/status.md (implementation ledger)
- [ ] user guide / administrator guide / provider troubleshooting — **not written**; the honest gaps live in the Setup Wizard panel and status.md instead

## Pre-flight (all require the real machine / CI)
- [ ] Clean-machine install of the signed package (S27.4, S27.39)
- [ ] Updater accepts the correctly signed package and rejects a mismatched archive/signature (S27.31)
- [ ] Adapter contract tests against live CLI versions (S27.32)
- [ ] §20's 20-step end-to-end demonstration

See `docs/security/release-signing.md` for the exact trust model, secret names,
candidate workflow, clean-machine checks, and publication gate. The build-only
`desktop-build.yml` artifact is explicitly non-distributable.
