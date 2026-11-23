# Project context

## Purpose and requirements

The WT32-ETH01 bridges the Qvantum QE-4 heat pump's Ethernet port to a
2.4 GHz Wi-Fi network. The pump should appear on its existing IPv4 VLAN as a
single client with its own MAC address. DHCP, ARP and Modbus TCP must pass so
Home Assistant can reach it under the existing MikroTik firewall rules.
MikroTik remains responsible for routing, DHCP and filtering. The bridge
creates no IP interface, uses no NAT and has no normal-mode management IP.

The deployment needs continuous operation and unattended recovery from common
network outages and initialization failures. Keep the exposed services and
diagnostics minimal. The current firmware uses WPA2-Personal with credentials
compiled in via `WIFI_SSID` and `WIFI_PASS`. Use dummy values in CI and keep
real credentials and credential-bearing firmware private. [README](README.md)

## Repository and architecture

- This is the active Rust fork of Owen Walpole's bridge. The older C/C++
  project, forked from doctormord and built with PlatformIO and ESP-IDF, is a
  separate reference for provisioning, recovery, telemetry and OTA. Those
  features belong to the C/C++ project. [Rust upstream](https://github.com/owenthewizard/esp32-wifi-bridge) · [C/C++ fork](https://github.com/Hjalle7/ESP32-Wifi-Bridge-on-Steroids) · [C/C++ upstream](https://github.com/doctormord/ESP32-Wifi-Bridge-on-Steroids)
- The owner moves through `Idle → EthReady → WifiReady → Running`. It learns
  the source MAC from the first Ethernet RX frame with at least 12 bytes,
  clones it onto the Wi-Fi station and requests association. The learning
  frame is dropped. The MAC is held in memory for the firmware lifetime.
  The bridge forwards frames directly through the two RX callbacks with
  no application queue, packet worker or per-frame allocation. [Bridge state machine](src/bridge.rs)
- `Running` owns both drivers. Static callbacks use nonblocking gates and a
  published C Ethernet handle. Both TX APIs copy before returning. Each
  Ethernet RX allocation and Wi-Fi RX token must be released exactly once,
  including on drops and errors. Teardown closes and drains the gates,
  unregisters STA RX synchronously, clears the handle, stops Ethernet and
  joins its link-timer barrier, then drops Wi-Fi before Ethernet. [Callback contracts](SAFETY.md) · [Gates](src/callback_gate.rs)
- The ESP32 RMII RX task can outlive Ethernet uninstall. It dispatches via
  the static `stack_input_info` mediator, even with a null timestamp. The
  pinned SDK and svc combination retains the MAC, PHY, DMA and RX task, so
  Ethernet driver recreation is unsupported. STA RX is registered directly.
  Keep the TX-done callback unregistered while the svc wrapper misinterprets
  its length pointer. Recheck these version-specific contracts when changing
  SDK, target, driver versions or Wi-Fi modes. [Ethernet mediator](src/ethernet_rx.rs) · [SDK safety review](SAFETY.md)

## Hardware and flash

- Target WT32-ETH01. The V1.4 bench unit is an ESP32-D0WD-V3 rev. 3.1 with
  8 MiB physical SPI flash. Factory boot, CP2102 communication, flash
  identification and backup are recorded. Account for 4 MiB variants and
  verify the actual module, flash capacity and wiring before deployment.
  The LAN87XX PHY uses address 1, MDC GPIO23, MDIO GPIO18, external 50 MHz
  RMII clock on GPIO0 and PHY reset/oscillator enable on GPIO16. The full
  pinout and bench procedure are in the [README](README.md).
- The checked image uses a 2 MiB flash header with DIO at 40 MHz. The factory
  app slot is 1 MiB at `0x10000` and ends at `0x110000` (exclusive). Physical
  flash capacity is independent of the image header and partition layout.
  The SDK defaults configure 30 RX and 30 TX Ethernet DMA buffers of
  1,522 bytes each. [Flash layout](README.md#build-and-check-the-flash-images) · [SDK defaults](sdkconfig.defaults)

## Toolchain and dependencies

- Active baseline is ESP-IDF v6.1.1 (`051e4c4e`) with ESP Rust 1.97.0.0
  installed as `esp`, target `xtensa-esp32-espidf` and linker `ldproxy`.
  CI pins the ESP Rust release. Cargo pins the SDK tag for SDKs it manages.
  Follow the activation checks in the Codex environment when building.
  [Cargo config](.cargo/config.toml) · [Rust toolchain](rust-toolchain.toml) · [Agent instructions](AGENTS.md)
- The Cargo lockfile resolves `esp-idf-svc 0.53.0`, `esp-idf-hal 0.47.0`,
  `esp-idf-sys 0.38.1`, `embuild 0.33.7` and `libc 0.2.185`. ESP-IDF 6
  uses the external Espressif LAN87XX v1.1.0 component pinned at `b30d1375`.
  [Manifest](Cargo.toml) · [Cargo lockfile](Cargo.lock) · [Component lockfile](components_esp32.lock)
- [PR #8](https://github.com/Hjalle7/esp32-wifi-bridge/pull/8) added a
  temporary C linker shim because svc 0.53.0 leaves
  `wifi_task_stack_size` at zero. It takes the SDK's full
  `WIFI_INIT_CONFIG_DEFAULT()` and preserves svc's NVS enable decision.
  The SDK's configured stack size is 6,656 bytes. Remove the C component,
  its manifest entry and the wrap flag when adopting a compatible svc
  release containing the merged upstream fix. Recheck the resolved
  HAL/sys and Ethernet changes, initialization and teardown contracts,
  image fit, and Wi-Fi startup and association on hardware before accepting
  the change. [Shim](components/wifi_init/wifi_init.c) · [Linker hook](build.rs) · [Upstream fix](https://github.com/esp-rs/esp-idf/pull/610)
- ESP-IDF v5.3.3 / ESP Rust 1.85.0.0 is the historical Rust reference.
  The [baseline tag](https://github.com/Hjalle7/esp32-wifi-bridge/tree/baseline/idf-5.3.3)
  points to `0ea3ef8`, the parent of the PR #8 squash commit. The older
  image measurements were taken before the TX diagnostics. [Historical checks](SAFETY.md#historical-esp-idf-v533-findings)

## Verified status

- PR #2 corrected callback ownership and avoided the defective Wi-Fi TX
  wrapper. PR #6 added lightweight UART TX diagnostics. PR #7 documented
  Codex SDK activation and recovery. PR #8 completed the SDK upgrade and
  added the initialization shim and CI image checks. It was squash-merged
  to `master` as `e25215c`, with the same tree as PR head `da8dab1`.
  The 21 November 2026 handover described the upgrade as pending.
  [PR #2](https://github.com/Hjalle7/esp32-wifi-bridge/pull/2) · [PR #6](https://github.com/Hjalle7/esp32-wifi-bridge/pull/6) · [PR #7](https://github.com/Hjalle7/esp32-wifi-bridge/pull/7) · [Merge commit](https://github.com/Hjalle7/esp32-wifi-bridge/commit/e25215ce44214607760e13ef84e07a5e22e9b0fb)
- Post-merge CI passed formatting, warnings-as-errors Clippy and a locked
  ESP32 release build with dummy credentials. All 14 host regressions passed
  (8 callback gate, 3 RMII mediator ABI stubs and 3 TX diagnostics).
  The release job passed the factory app and bootloader size checks.
  [CI run](https://github.com/Hjalle7/esp32-wifi-bridge/actions/runs/45125566725) · [Workflow](.github/workflows/rust_ci.yml)
- The post-merge CI image was **962,048 bytes**, leaving **86,528 bytes**
  in the factory slot. The bootloader was **25,904 bytes**, leaving **2,768
  bytes** before the partition table. The merged README records a local
  Linux image of **963,008 bytes** with **85,568 bytes** free. The PR body
  quotes **962,864 bytes** with **85,712 bytes** free and links earlier CI
  that predates the shim and automated size checks. The reason for the
  size differences is unconfirmed. Use the image measured from the exact
  build being assessed. [Post-merge size log](https://github.com/Hjalle7/esp32-wifi-bridge/actions/runs/45125566725/job/137528996473) · [README](README.md#build-and-check-the-flash-images) · [PR #8](https://github.com/Hjalle7/esp32-wifi-bridge/pull/8)
- The Rust firmware still needs physical acceptance on WT32-ETH01 and
  end-to-end validation with the QE-4. The host stubs, cross-build and image
  checks establish software and image properties. The board checks above
  cover the factory firmware and programming interface. [Acceptance procedure](README.md#physical-acceptance-checks)

## Open work and risks

- Verify boot and association with the cloned MAC, DHCP/ARP, bidirectional
  IPv4 and Modbus TCP from Home Assistant. Test sustained traffic, Ethernet
  and Wi-Fi loss under load, reboot and teardown with RX/TX in flight. Use
  the TX counters to investigate possible DMA descriptor exhaustion. The
  counters measure API outcomes, so correlate them with packet loss before
  changing the DMA ring. Verify enclosure RF performance, antenna needs
  and a suitable power supply. [Bench procedure and diagnostics](README.md)
- Define bounded Wi-Fi reconnect, initialization recovery and watchdog
  behavior. Currently startup failures can abort, Wi-Fi is asked to connect
  once, and the main loop only reports diagnostics. Decide how to detect
  a stuck datapath versus an ordinary outage, what warrants a device reset
  and whether MAC persistence or a client replacement procedure is needed.
  Keep recovery compatible with the firmware-lifetime RMII ownership.
  [Startup and main loop](src/main.rs) · [Driver lifetimes](SAFETY.md)
- Decide whether field provisioning is needed and how to authorize access
  and restore service after credential changes. Ethernet HTTP provisioning
  is a candidate. The compiled credentials remain the current setup.
  Any provisioning, MQTT or OTA work requires a separate decision and
  should preserve the absence of a normal-mode management IP. [TODO](TODO.md)
- Confirm the production flash header and partition layout for the board
  variants in scope. Recheck factory-slot headroom as features or dependencies
  grow. Maintain both lockfiles, review stable upstream releases and relevant
  advisories, and check the SDK-specific callback contracts and image budget
  when upgrading. [Flash checks](README.md#build-and-check-the-flash-images) · [Safety review](SAFETY.md)

## Development workflow

Use small PRs against `master`. Review unsafe code, FFI, driver lifetimes and
recovery changes independently. For any validation report, record source
inspection, host tests, build and image checks, and physical results
separately. Update this document when architecture, toolchain baselines or
major project status change. Keep detailed procedures and contracts in their
linked files. [Agent instructions](AGENTS.md) · [Host test commands](SAFETY.md#regression-checks-and-hardware-limits)
