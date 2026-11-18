# Forwarding callback safety

This applies to the locked `esp-idf-svc 0.50.1`, `esp-idf-hal 0.45.0`,
`esp-idf-sys 0.36.0`, and ESP-IDF `v5.3.3`. Keep the single-client MAC-cloning
bridge and its `Idle -> EthReady -> WifiReady -> Running` lifecycle. No IP
interface or additional network service is created.

## Confirmed defects

The previous callbacks converted integer-cast pointers into `&mut EthDriver`
and `&mut WifiDriver`. A stable heap address did not justify those exclusive
borrows: callbacks could overlap driver method borrows, another callback, or
destruction. The original field order also destroyed Ethernet while the Wi-Fi
callback could still use it. Replacing the Ethernet receive closure after MAC
learning relied on `stop()` quiescing a task; it does not join the RMII RX task.

In `esp-idf-svc 0.50.1`, `WifiDriver::handle_tx` passes `len as usize` to
`slice::from_raw_parts`, although ESP-IDF declares `len` as `uint16_t *`.
This constructs an invalid slice **before** invoking the user's empty callback.
The old `set_nonstatic_callbacks` registered this wrapper, so Wi-Fi transmission
made the path reachable. This is undefined behavior even without reading the
resulting slice.

`0.50.1` is the latest published `0.50.x` release. The same defect is present
in `0.51.0`, `0.52.1`, `0.53.0`, and the
maintained monorepo at
[`21825fe8e6e08270e27eacbacca0be38a3b4c164`](https://github.com/esp-rs/esp-idf/blob/21825fe8e6e08270e27eacbacca0be38a3b4c164/esp-idf-svc/src/wifi.rs).
There is no fixed compatible release among these versions. Updating would not
resolve it; vendoring the crate would add unnecessary maintenance here.
Instead, register **only** the station RX function with
`esp_wifi_internal_reg_rxcb`. Neither `set_callbacks` nor
`set_nonstatic_callbacks` nor a TX-done callback is installed. The defective
wrapper remains in the dependency, but is unreachable in this application.
Do not reintroduce either combined callback API without fixing that dependency.
`Cargo.toml`, `Cargo.lock`, and ESP-IDF remain unchanged.

## Ownership, concurrency, and teardown

- Both receive callbacks are static C functions. They capture no driver,
  closure environment, stack borrow, or integer-disguised Rust pointer. The
  driver's Rust address is irrelevant; the only published pointer is the opaque
  C Ethernet handle, stored in an `AtomicPtr`.
- `Peripherals::take()` and Wi-Fi enforce a single driver instance. The static
  MAC cell and gates belong to that singleton's lifetime. The MAC cell keeps
  only the first Ethernet source MAC. Register Ethernet's input path once,
  before its first start; use that same callback for learning and forwarding.
- Each direction has a separate atomic gate. A successful acquire admits only
  one transmitter in that direction and observes the owner's release of the
  initialized handle. Concurrent/reentrant admission fails immediately and
  discards that frame. Opposite directions can proceed concurrently. No Rust
  driver reference is ever constructed by a callback. Ethernet link state and
  Wi-Fi readiness are checked by the C transmit APIs via their error returns.
- `Running` owns both drivers **before** Wi-Fi RX registration or fallible
  starts. Gates open only after initialization/start/connect calls succeed.
  Thus setup failures cannot leave forwarding enabled into a destroyed driver.
  Earlier states have closed gates and only the static Ethernet callback.
- Drop first closes both gates atomically, then waits for admitted callbacks
  to release their guards. Guard release clears only the active bit and cannot
  reopen a closed gate. Acquire observation of the idle state completes draining
  before the handle is cleared or either driver is destroyed. Only the owner
  waits; callbacks never wait for it. Ownership cannot be dropped from inside
  these callbacks because neither callback owns or accesses `Running`.
- Fields then drop in declaration order: Wi-Fi first (its `clear_all` stops and
  deinitializes Wi-Fi, which unregisters RX), then Ethernet (stop/uninstall).
  A late callback rejected by a gate only consumes its RX buffer; it cannot
  access either transmit driver or a freed Rust callback environment. No live
  callback is replaced, and no application callback storage must be freed.
  Forgetting the bridge leaks its drivers together, keeping its handles valid.

This relies on ESP-IDF's supported stop/deinit/uninstall operations managing
their own tasks, DMA, and buffers. It does not claim to prove the SDK's internal
implementation safe or that `esp_eth_stop` joins the RX task. In particular,
the safety argument does not use stop alone to free or replace callback state.

## Buffers and remaining unsafe calls

The Ethernet RX task supplies a malloc-allocated owned buffer. The callback
copies just the six source-MAC bytes after checking `len >= 12`, transmits if
admitted, and calls `free` exactly once, including rejected/error frames.
Lengths that do not fit Wi-Fi's `uint16_t` are discarded rather than truncated.

Wi-Fi supplies the RX buffer and its release token `eb`. The callback forwards
synchronously when admitted, then calls `esp_wifi_internal_free_rx_buffer(eb)`
exactly once, including rejected/error frames, and returns `ESP_OK` to mark
the buffer consumed. No Rust slice is constructed from either C RX buffer.

ESP-IDF's `esp_wifi_internal_tx` explicitly copies its input before returning.
For this board, `esp_eth_transmit` dispatches to the ESP32 RMII MAC, whose
`emac_esp_dma_transmit_frame` copies into DMA buffers before returning. Neither
transmitter retains the input RX buffer. These existing driver copies are
required; the bridge adds no frame copy, queue, heap allocation, blocking lock,
or retry loop to steady-state callbacks. MAC learning/log formatting happens
once. The only added waiting loop is in owner teardown.

Relevant inspected source contracts:

- [ESP-IDF v5.3.3 Wi-Fi private API](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_wifi/include/esp_private/wifi.h):
  `wifi_rxcb_t`, `esp_wifi_internal_tx`, `esp_wifi_internal_free_rx_buffer`,
  `wifi_tx_done_cb_t`.
- [ESP-IDF v5.3.3 Ethernet driver](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_eth/src/esp_eth.c):
  input-path registration, transmit, stop, uninstall.
- [ESP32 RMII MAC](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_eth/src/mac/esp_eth_mac_esp.c)
  and [DMA](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_eth/src/mac/esp_eth_mac_esp_dma.c):
  single RX task and synchronous transmit copying.
- [esp-idf-svc 0.50.1 Wi-Fi](https://github.com/esp-rs/esp-idf-svc/blob/v0.50.1/src/wifi.rs)
  and [Ethernet](https://github.com/esp-rs/esp-idf-svc/blob/v0.50.1/src/eth.rs):
  callback registration, RX ownership/freeing, and driver destructors.

## Regression checks and hardware limits

The admission/draining primitive has host tests, including concurrent senders,
close during an active callback, and a close/admission race. Run independently
of the ESP binary (`harness = false`) and its target build configuration:

```sh
rustc +stable --edition=2021 --test tests/host/callback_gate.rs -o /tmp/bridge-callback-tests
/tmp/bridge-callback-tests
```

These tests exercise the actual atomic gate implementation, not ESP-IDF or the
physical drivers. Separately run the repository's formatting, Clippy, and release
build commands with dummy `WIFI_SSID` / `WIFI_PASS`. Compilation does not prove
runtime memory safety. WT32-ETH01 checks still need real hardware: MAC cloning,
bidirectional load, DHCP/ARP pass-through, link loss during traffic, startup
failures, and dropping the bridge with RX/TX in flight. Confirm the release image
fits the configured partition on a 4 MB device; never commit generated firmware.
