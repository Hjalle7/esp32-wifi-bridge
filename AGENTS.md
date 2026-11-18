# Development guidelines

## Scope and architecture

- These instructions apply only to `Hjalle7/esp32-wifi-bridge` (Rust).
- Prioritize reliability, security, simplicity, and maintainability.
- Preserve the single-client Ethernet-to-WiFi forwarding architecture in `src/bridge.rs`: learn the source MAC from the first Ethernet frame, clone it onto the WiFi station, and forward frames in both directions. Preserve the `Idle -> EthReady -> WifiReady -> Running` lifecycle; do not introduce routing, NAT, or multi-client behavior without an explicit request.
- Target WT32-ETH01 (ESP32, 4 MB flash). Respect the existing LAN87XX RMII pin, clock, and PHY configuration and account for flash/RAM limits.

## Build context

- Follow Rust 2021, rustfmt, and the existing Clippy lints.
- Use the `esp` Rust toolchain, `xtensa-esp32-espidf` target, `ldproxy`, and ESP-IDF `v5.3.3` configured in `rust-toolchain.toml` and `.cargo/config.toml`.
- Review dependency safety and compatibility with ESP-IDF and the target, including the `embedded-svc` Git patch and locked revisions. Avoid unrelated dependency or lockfile changes.
- Prefer the newest stable, supported, compatible dependencies, toolchains, and frameworks. Evaluate upgrades for compatibility, security, reliability, maintainability, and hardware constraints; replace obsolete workarounds when newer versions offer safer or simpler solutions, regardless of prior implementation effort. Use prereleases only with explicit approval.
- `WIFI_SSID` and `WIFI_PASS` are compile-time environment variables. Supply real values locally; never commit credentials, secrets, credential-bearing firmware, or generated build artifacts (including `target/`, `.embuild/`, and generated SDK configuration).

## Reliability and safety

- Require automatic WiFi reconnection, watchdog supervision, and recovery from initialization failures in runtime development. These are development requirements, not assertions that the current code implements them: startup uses `expect`/`unwrap`, WiFi connects once, and `main` only idles afterward; application-level recovery and watchdog supervision are not implemented in the inspected source. Address gaps within the requested task's scope; do not add functionality to documentation-only changes.
- Review every unsafe block and callback change for ownership, buffer lifetimes, stable driver addresses, callback registration/unregistration and drop order, aliasing, and concurrent access. The current forwarding callbacks capture raw driver pointers as integers; heap allocation alone does not establish concurrency safety. Document safety reasoning and check dependency callback guarantees.
- Minimal UART diagnostics, including RSSI and connection/disconnection events, are allowed. Keep them lightweight and free of secrets. `log` currently uses `release_max_level_off`; do not claim release diagnostics exist without verifying the configuration.

## Feature boundaries

- Do not add MQTT, OTA, a permanent management IP, or WiFi provisioning unless explicitly requested.
- Ethernet-based provisioning is a possible future feature listed in `TODO.md`, not an implemented feature or a current requirement.

## Validation and delivery

- Keep changes focused and add or run relevant tests when behavior changes. The binary uses `harness = false`; no automated test suite is currently present. Do not report linting or compilation as tests.
- With the ESP toolchain and suitable environment values, follow the existing CI checks: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --workspace -- -D warnings`, and `cargo build --release`. Use dummy credentials for automated checks.
- Report source inspection, automated tests, compilation, and physical hardware validation separately, including skipped checks and their reasons. Compilation cannot establish forwarding, reconnection, watchdog, or recovery behavior. State explicitly when WT32-ETH01 hardware validation was unavailable. `cargo run --release` flashes hardware via the configured runner; it is not a build-only check.
- Work on a dedicated branch and open a pull request against `master`. Never merge automatically or enable auto-merge. Confirm the final diff contains only the requested files.
