# Development guidelines

## Scope and architecture

- These instructions apply only to `Hjalle7/esp32-wifi-bridge` (Rust).
- Prioritize reliability, security, simplicity, and maintainability.
- Preserve the single-client Ethernet-to-WiFi forwarding architecture in `src/bridge.rs`: learn the source MAC from the first Ethernet frame, clone it onto the WiFi station, and forward frames in both directions. Preserve the `Idle -> EthReady -> WifiReady -> Running` lifecycle; do not introduce routing, NAT, or multi-client behavior without an explicit request.
- Target WT32-ETH01 (ESP32, including 4 MB boards and the verified 8 MB V1.4 unit). Respect the existing LAN87XX RMII pin, clock, and PHY configuration and account for flash/RAM limits.

## Build context

- Follow Rust 2021, rustfmt, and the existing Clippy lints.
- Use the `esp` Rust toolchain, `xtensa-esp32-espidf` target, `ldproxy`, and ESP-IDF `v5.3.3` configured in `rust-toolchain.toml` and `.cargo/config.toml`.
- Review dependency safety and compatibility with ESP-IDF and the target, including locked versions and sources. Avoid unrelated dependency or lockfile changes.
- Prefer the newest stable, supported, compatible dependencies, toolchains, and frameworks. Evaluate upgrades for compatibility, security, reliability, maintainability, and hardware constraints; replace obsolete workarounds when newer versions offer safer, simpler, or more maintainable solutions, regardless of prior implementation effort. Use prereleases only with explicit approval.
- `WIFI_SSID` and `WIFI_PASS` are compile-time environment variables. Supply real values locally; never commit credentials, secrets, credential-bearing firmware, or generated build artifacts (including `target/`, `.embuild/`, and generated SDK configuration).

## Codex environment selection and recovery

- The repository currently pins ESP-IDF v5.3.3 and ESP Rust 1.85.0.0. Use those pinned versions for baseline builds unless the task explicitly requests the coordinated ESP-IDF 6.1.1 / ESP Rust 1.97.0.0 upgrade. Do not silently mix toolchains or change project pins to match an environment.
- For upgrade work in the Codex environment named `IDF 6.1.1`, use the isolated installation at `/workspace/.esp32-idf611-rust197`. Before any build or source changes, source `/workspace/.esp32-idf611-rust197/activate.sh` in the shell that will run the build and verify `idf.py --version`, `rustc +esp --version`, and the ESP-IDF Python dependency check.
- If activation fails specifically because the ESP-IDF 6.1 Python constraints or packages are missing, retry only the official `python3 "$IDF_PATH/tools/idf_tools.py" --non-interactive install-python-env` with `IDF_PATH=/workspace/.esp32-idf611-rust197/esp-idf-v6.1.1` and `IDF_TOOLS_PATH=/workspace/.esp32-idf611-rust197/idf-tools` exported. Keep TLS verification and dependency constraints enabled. Recheck dependencies and activation in a fresh shell. Do not reinstall working Rust/SDK tools, modify the persistent environment setup, or fall back to the legacy toolchain without approval.
- Persistent Codex setup has intermittently received proxy CONNECT 403 for `dl.espressif.com:443` even when the domain is allowlisted; a manual Python install in a normal session has succeeded. Treat this as an environment-provisioning limitation, not a project code defect. If repair fails, stop before modifying project files and report the setup log/error. A successful manual repair does not prove unattended provisioning.
- Do not apply this Codex-specific recovery procedure to the legacy ESP-IDF 5.3.3 environment or local developer machines.

## Reliability and safety

- Require automatic WiFi reconnection, watchdog supervision, and recovery from initialization failures in runtime development. These are development requirements, not assertions that the current code implements them: startup uses `expect`/`unwrap`, WiFi connects once, and `main` only idles afterward; application-level recovery and watchdog supervision are not implemented in the inspected source. Address gaps within the requested task's scope; do not add functionality to documentation-only changes.
- Review every unsafe block and callback change for ownership, buffer lifetimes, stable driver addresses, callback registration/unregistration and drop order, aliasing, and concurrent access. Callbacks use static gates, a static RMII MAC mediator, and an atomic C Ethernet handle; they do not capture raw Rust driver pointers or reconstruct driver references. Preserve RX-only WiFi registration (avoiding the defective svc TX wrapper), gate draining, explicit WiFi RX unregister before destruction, and WiFi-before-Ethernet drop order. Follow the version-specific contracts and retained RMII resource limitation in `SAFETY.md`; gates alone do not join RX callbacks.
- Minimal UART diagnostics, including RSSI and connection/disconnection events, are allowed. Keep them lightweight and free of secrets. `log` currently uses `release_max_level_warn` for the hardware baseline. Keep packet callbacks free of logging.

## Feature boundaries

- Do not add MQTT, OTA, a permanent management IP, or WiFi provisioning unless explicitly requested.
- Ethernet-based provisioning is a possible future feature listed in `TODO.md`, not an implemented feature or a current requirement.

## Validation and delivery

- Keep changes focused and add or run relevant tests when behavior changes. The binary uses `harness = false`; run the 8 callback-gate and 3 static-mediator ABI-stub host regression tests with the standalone `rustc +stable --test` commands in `SAFETY.md`. CI also runs 3 TX diagnostics host tests; preserve and run these when touching TX reporting. These host suites do not exercise physical drivers; do not report linting or compilation as tests.
- With the ESP toolchain and suitable environment values, follow the existing CI checks: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features --workspace -- -D warnings`, and `cargo build --release --locked`. Use dummy credentials for automated checks.
- The historical PR #2 baseline passed formatting, warnings-as-errors Clippy, all 11 host tests, and release compilation; `SAFETY.md` records the local Rust 1.85.0.0 image at 765,584 bytes in a 1 MiB factory partition. Recheck the actual image and partition budget after dependency changes; record the toolchain when comparing sizes. Hardware validation remains outstanding.
- Report source inspection, automated tests, compilation, and physical hardware validation separately, including skipped checks and their reasons. Compilation cannot establish forwarding, reconnection, watchdog, or recovery behavior. State explicitly when WT32-ETH01 hardware validation was unavailable. `cargo run --release` flashes hardware via the configured runner; it is not a build-only check.
- Work on a dedicated branch and open a pull request against `master`. Never merge automatically or enable auto-merge. Confirm the final diff contains only the requested files.
