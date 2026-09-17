# NACC release checklist (master plan S24 Phase 12)

Every line states its evidence requirement. Items marked **[CI/signed]** need
a signing certificate, a Windows SDK/MSVC environment, or CI dispatch — none
of which exists in this working tree, so they remain unchecked here on
purpose. Do not tick them without the evidence.

## Build
- [ ] `cargo +1.96.0-x86_64-pc-windows-gnu clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo +1.96.0-x86_64-pc-windows-gnu test --workspace`
- [ ] `npm run build && npm test`
- [ ] `cargo +1.96.0-x86_64-pc-windows-gnu run -p nacc-app -- --export-bindings` (bindings fresh)
- [ ] **[CI/signed]** `tauri build` MSVC target produces the NSIS installer
- [ ] **[CI/signed]** installer + updater artifacts signed (certificate required)

## Docs shipped in-repo (this build)
- [x] docs/security/threat-model.md
- [x] docs/architecture/adr-0001-adapter-contract-deferrals.md
- [x] docs/architecture/status.md (implementation ledger)
- [ ] user guide / administrator guide / provider troubleshooting — **not written**; the honest gaps live in the Setup Wizard panel and status.md instead

## Pre-flight (all require the real machine / CI)
- [ ] Clean-machine install of the signed package (S27.4, S27.39)
- [ ] Updater verifies a signed package (S27.31)
- [ ] Adapter contract tests against live CLI versions (S27.32)
- [ ] §20's 20-step end-to-end demonstration
