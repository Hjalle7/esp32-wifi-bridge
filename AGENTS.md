# Development guidelines

## Project context

- Read [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md) before substantial development work.
- Verify current code and locked dependencies against the context before relying on it.
- Update `PROJECT_CONTEXT.md` when architectural decisions, toolchain baselines, or major project status change. Keep `AGENTS.md` focused on agent instructions and record project history in `PROJECT_CONTEXT.md`.

## Scope and architecture

- These instructions apply only to `Hjalle7/esp32-wifi-bridge` (Rust).
- Prioritize reliability, security, simplicity, and maintainability.
- Preserve the single-client Ethernet-to-WiFi forwarding architecture in `src/bridge.rs`: learn the source MAC from the first Ethernet frame, clone it onto the WiFi station, and forward frames in both directions. Preserve the `Idle -> EthReady -> WifiReady -> Running` lifecycle; do not introduce routing, NAT, or multi-client behavior without an explicit request.
- Target WT32-ETH01 (ESP32, including 4 MB boards and the verified 8 MB V1.4 unit). Respect the existing LAN87XX RMII pin, clock, and PHY configuration and account for flash/RAM limits.

## Build context

- Follow Rust 2021, rustfmt, and the existing Clippy lints.
- Use ESP Rust `1.97.0.0` installed as `esp`, the `xtensa-esp32-espidf` target, `ldproxy`, and ESP-IDF `v6.1.1`. CI pins the Rust release because `rust-toolchain.toml` selects the locally installed `esp` alias. `.cargo/config.toml` pins the SDK tag for builds that let Cargo obtain the SDK.
- Review dependency safety and compatibility with ESP-IDF and the target, including locked versions and sources. Avoid unrelated dependency or lockfile changes.
- Prefer the newest stable, supported, compatible dependencies, toolchains, and frameworks. Evaluate upgrades for compatibility, security, reliability, maintainability, and hardware constraints; replace obsolete workarounds when newer versions offer safer, simpler, or more maintainable solutions, regardless of prior implementation effort. Use prereleases only with explicit approval.
- `WIFI_SSID` and `WIFI_PASS` are compile-time environment variables. Supply real values locally; never commit credentials, secrets, credential-bearing firmware, or generated build artifacts (including `target/`, `.embuild/`, and generated SDK configuration).

## Codex environment selection and recovery

- In the Codex environment named `IDF 6.1.1`, use the isolated installation at `/workspace/.esp32-idf611-rust197`. Before any build or source changes, source `/workspace/.esp32-idf611-rust197/activate.sh` in the build shell. Verify `idf.py --version`, `rustc +esp --version`, `python "$IDF_PATH/tools/idf_tools.py" check-python-dependencies`, and availability of the Xtensa tools, Clang/libclang, `ldproxy`, CMake and Ninja. Set `ESP_IDF_TOOLS_INSTALL_DIR=fromenv` to require Cargo to use the activated SDK and tools.
- If activation fails specifically because the ESP-IDF 6.1 Python constraints or packages are missing, retry only the official `python3 "$IDF_PATH/tools/idf_tools.py" --non-interactive install-python-env` with `IDF_PATH=/workspace/.esp32-idf611-rust197/esp-idf-v6.1.1` and `IDF_TOOLS_PATH=/workspace/.esp32-idf611-rust197/idf-tools` exported. Keep TLS verification and dependency constraints enabled. Recheck dependencies and activation in a fresh shell. Do not reinstall working Rust/SDK tools, modify the persistent environment setup, or fall back to the legacy toolchain without approval.
- Persistent Codex setup has intermittently received proxy CONNECT 403 for `dl.espressif.com:443` even when the domain is allowlisted; a manual Python install in a normal session has succeeded. Treat this as an environment-provisioning limitation, not a project code defect. If repair fails, stop before modifying project files and report the setup log/error. A successful manual repair does not prove unattended provisioning.
- Do not apply this Codex-specific recovery procedure to the legacy ESP-IDF 5.3.3 environment or local developer machines.

## Reliability and safety

- Require automatic WiFi reconnection, watchdog supervision, and recovery from initialization failures in runtime development. These are development requirements, not assertions that the current code implements them: startup uses `expect`/`unwrap`, WiFi connects once, and `main` only idles afterward; application-level recovery and watchdog supervision are not implemented in the inspected source. Address gaps within the requested task's scope; do not add functionality to documentation-only changes.
- Review every unsafe block and callback change for ownership, buffer lifetimes, stable driver addresses, callback registration and unregistration, drop order, aliasing and concurrent access. Preserve the static RMII `stack_input_info` mediator, atomic gates and C Ethernet handle. Keep RX-only Wi-Fi registration because svc 0.53.0 still has the TX wrapper defect. Preserve gate draining, explicit Wi-Fi RX unregister, the Ethernet link-timer barrier and Wi-Fi-before-Ethernet destruction. Follow the version-specific contracts and retained RMII resource limitation in `SAFETY.md`.
- Minimal UART diagnostics, including RSSI and connection/disconnection events, are allowed. Keep them lightweight and free of secrets. `log` currently uses `release_max_level_warn` for the hardware baseline. Keep packet callbacks free of logging.

## Feature boundaries

- Do not add MQTT, OTA, a permanent management IP, or WiFi provisioning unless explicitly requested.
- Ethernet-based provisioning is a possible future feature listed in `TODO.md`, not an implemented feature or a current requirement.

## Validation and delivery

- Keep changes focused and run the standalone `rustc +stable --test` commands in `SAFETY.md` for all 14 host tests. CI runs the callback-gate, static-mediator ABI-stub and TX diagnostics suites. The ESP binary uses `harness = false`.
- With the ESP toolchain and suitable environment values, follow the existing CI checks: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features --workspace -- -D warnings`, and `cargo build --release --locked`. Use dummy credentials for automated checks.
- Recheck the application image and the 1 MiB factory partition budget after dependency changes. Record the toolchain and commit when comparing sizes. Historical ESP-IDF v5.3.3 sizes are recorded in `SAFETY.md` and `README.md`.
- Report source inspection, automated tests, compilation, and physical hardware validation separately, including skipped checks and their reasons. Compilation cannot establish forwarding, reconnection, watchdog, or recovery behavior. State explicitly when WT32-ETH01 hardware validation was unavailable. `cargo run --release` flashes hardware via the configured runner; it is not a build-only check.
- Work on a dedicated branch and open a pull request against `master`. Never merge automatically or enable auto-merge. Confirm the final diff contains only the requested files.
