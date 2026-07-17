# Roadmap

v0.1 Slack — done (Phase 6, `step6.md`): Bot Token auth, polling (messages + a configured presence watch-list), `SendSlackMessage`.
v0.2 Presence — folded into v0.1 above (same adapter, same domain model; see `step6.md` Context for why splitting them would have meant building the same adapter twice).
v0.3 GitHub
v0.4 Calendar
v0.5 Plugins
v1.0 Stable + public release with prebuilt binaries (Windows/macOS/Linux) — see product-requirements.md §2.1, §4

Note: the release pipeline itself (cargo-dist, ADR-0015) is already built and validated via pre-release tags, ahead of the v1.0 line above — it was deliberately built early (against the pre-TUI skeleton) so packaging/signing/CI issues surface before they're tangled up with feature work, not because prebuilt binaries are meant to ship publicly before v1.0.
