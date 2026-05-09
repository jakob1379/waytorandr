# Verified Security TODO

This list replaces the inflated review dump with council-verified work. Items are ranked by practical risk, not by how scary the wording sounds.

## P0 — fix first

- [x] SR-001 Block zero-enabled-real-output plans before apply
  - Status: real threat.
  - Do: reject plans that disable every real output before backend apply. No CLI or daemon path should allow blank layouts. Internal-only blank topologies use an immutable built-in fallback before user defaults.
  - Tests: `off`/profile blank plan rejected by default; daemon skips blank matched/default/remembered layouts and applies built-in fallback for blank internal-only topology.

- [x] SR-002 Require strong identity for unattended auto-apply
  - Status: real threat.
  - Do: stop treating connector-only or connector-cache-enriched identity as sufficient for daemon/default/auto selection. Manual explicit profile apply can remain permissive.
  - Tests: connector-only match is refused for auto/default/daemon selection; cached connector identity does not upgrade trust; `set auto` selects and applies through raw live topology, not cached normalization.

## P1 — real safety issues

- [x] SR-004 Mark weak KScreen identities and restrict risky auto-apply
  - Status: real threat, coupled to SR-002.
  - Do: classify KScreen identities without stable EDID/vendor/model/serial data as weak; require strong identity for unattended apply.

## Final blocker review

- [x] Oracle and council rechecks found no remaining P0/P1/P2 blockers after the security TODO pass; follow-up findings for Flatpak permissions and topology-bound bypasses were fixed and revalidated.
- [x] Validation passed: `nix develop -c cargo fmt --all`, `nix develop -c cargo test --workspace`, `nix build`, `nix build .#flatpak`, `./result/bin/waytorandr --help`, `./result/bin/waytorandr save --help`, `./result/bin/waytorandr set --help`, `./result/bin/waytorandr service run --help`, `./result/bin/waytorandrd --help`.

- [x] SR-005/SR-008 Treat hooks as trusted executable content and make daemon hook execution controllable
  - Status: real threat.
  - Do: document hooks as arbitrary commands; add explicit hook execution policy for daemon/service run and workflow apply paths.

- [x] SR-009 Gate apply when validation is unsupported
  - Status: real safety issue.
  - Do: require explicit force/policy opt-in when backend validation is unsupported, especially KScreen. Surface unsupported validation in human and JSON output.

## P2 — hardening worth doing after P0/P1

- [x] SR-003 Prune stale blank remembered layouts
  - Do: remove blank remembered setup entries during state load/migration and when daemon encounters them.

- [x] SR-006/SR-007/SR-018 Harden hook execution
  - Do: avoid stdout/stderr pipe deadlocks, kill hook process groups on timeout, clamp hook timeouts.
  - Tests: noisy hook exits; timeout kills descendants; invalid timeout is clamped/rejected.

- [x] SR-010 Escape terminal controls in human output and logs
  - Do: centralize escaping for CLI human output and daemon logs. Keep JSON raw via serde escaping.

- [x] SR-011/SR-012 Reduce packaging overclaims
  - Do: narrow Flatpak permissions where possible; document or rework Snap classic confinement.

- [x] SR-013 Cap profile/state file sizes and legacy migration count
  - Do: enforce byte limits before full-file parse and cap legacy migration file count.

- [x] SR-017 Add topology flapping backoff/log suppression
  - Do: add duplicate-log suppression and cooldown after repeated churn.

- [x] SR-019 Treat externally edited profiles as executable content
  - Do: add store permission/trust checks and hook-bearing-profile warnings/policy.

- [x] SR-021/SR-023 Harden KScreen command surface
  - Do: validate KScreen output names before `kscreen-doctor` argv construction; prefer/canonicalize absolute executable paths and restrict env overrides in daemon/service mode.

- [x] SR-022 Cap hostile backend topology data
  - Do: cap output count, modes per output, string lengths, scale/refresh ranges, dimensions, and coordinate bounds.

## P3/P4 — regression coverage, docs, and hygiene

- [x] SR-014 Harden XDG config/state file handling where practical
- [x] SR-015 Redact/gate monitor identifiers in human output
- [x] SR-016 Clarify virtual/ignored fingerprint semantics
- [x] SR-020 Tighten GNOME verify/apply contract
- [x] SR-025 Bound dynamic shell completion cost
- [x] SR-028 Add migration conflict tests and skipped-migration warnings
- [x] SR-029 Validate profile names for display safety
- [x] SR-030 Keep exact-match auto-apply behavior covered
- [x] SR-031 Keep setup-default scoping documented
- [x] SR-032 Preserve no-shell hook execution with tests/docs
- [x] SR-033 Cover wlroots serial retry behavior
- [x] SR-034 Keep `set` invalid combinations tested
- [x] SR-035 Keep JSON output centralized through serde
- [x] SR-036 Add service install quoting tests and PATH docs
- [x] SR-037 Preserve Home Manager absolute executable behavior
- [x] SR-038 Document PATH/XDG same-user trust boundaries
- [x] SR-039 Pin CI actions and keep dependency audit coverage
- [x] SR-040 Make CI job permissions explicit

## De-scoped as security threats

- SR-024: hostile backend strings do not become hooks or arbitrary paths. Keep only regression/display-sanitization coverage.
- SR-026: AUR `sha256sums=SKIP` is acceptable for VCS PKGBUILDs if source is pinned. Improve reproducibility later if desired.
- SR-027: debug-only test backend is build hygiene, not a release security flaw.
