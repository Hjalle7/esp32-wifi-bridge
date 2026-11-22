# Forwarding callback safety

This review applies to ESP32 with ESP-IDF v6.1.1 (commit
`051e4c4eb20346e043cdd47bc388fde75ff8af63`), ESP Rust 1.97.0.0,
`esp-idf-svc 0.53.0`, `esp-idf-hal 0.47.0`, `esp-idf-sys 0.38.1` and
Espressif's LAN87XX v1.1.0 component. The single-client bridge retains the
`Idle -> EthReady -> WifiReady -> Running` lifecycle and creates no IP
interface. Packet callbacks use static state and a C Ethernet handle.

## Ethernet dispatch and teardown

The ESP32 MAC creates one `emac_rx` task. In v6.1.1 it invokes the mediator's
four-argument `stack_input_info(eth, buffer, length, info)` synchronously for
every delivered frame. The `info` pointer is null on ESP32. On MACs that support
IEEE 1588, the driver can pass a pointer to a timestamp on the RX task's stack.
The bridge ignores this argument. Bindgen checks the callback's parameter
types and the layout of the full mediator, including the legacy `stack_input`
slot and the low-level state hook.
[ESP-IDF v6.1.1 mediator header](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_eth/include/esp_eth_com.h)
and [ESP32 RMII MAC](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_eth/src/mac/esp_eth_mac_esp.c).

The default `eth_stack_input_info` reads the driver-owned mediator before
calling the configured input function, with no RX-side driver reference.
The ordinary and extended input-path setters assign fields with no callback
join. `esp_eth_stop` stops hardware through the PHY/MAC link path but has no
RX-task join. Uninstall deinitializes the PHY and MAC and frees the Ethernet
driver, including its mediator. The RX task can still run. The svc 0.53.0
`EthDriver` destructor stops and uninstalls. `RmiiEth` is empty, and the
destructor leaves the PHY, MAC, DMA and RX task allocated. The MAC's `del`
method deletes the task and frees resources. FreeRTOS task deletion provides
no cooperative callback join.
[ESP-IDF v6.1.1 Ethernet driver](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_eth/src/esp_eth.c)
and [svc 0.53.0 Ethernet ownership](https://github.com/esp-rs/esp-idf-svc/blob/d42a67836f965eb6fd5d2b04785ab294751c6aa7/src/eth.rs).

`ethernet_rx::install` looks up the MAC on the initialized, never-started
driver. The ESP32 MAC's `set_mediator` just stores the pointer. The installed
mediator has static storage, dispatches directly to the Rust callback and
ignores the low-level init/deinit notices, which have no user hooks in the
svc constructor. The PHY keeps its original mediator for SMI and link state.
The ESP32 build has no timestamp ISR dispatch. A late RX callback can access
the static mediator, learn a MAC if needed and free its buffer. A closed
forwarding gate prevents it from accessing Wi-Fi. This adapter is specific to
the ESP32 RMII MAC.

The PHY's periodic link check is a separate callback on the serialized
`ESP_TIMER_TASK`. The driver's `esp_timer_stop` disarms the timer, and a
callback already dispatched can still dereference the driver and its PHY
mediator. `stop_ethernet` schedules a one-shot on the same task after disarming
the link timer and waits for its release/acquire completion signal. The owner
then joins the one-shot itself using `esp_timer_stop_blocking` before its Rust
callback storage is dropped. This runs after MAC learning and before Ethernet
destruction. It adds a timer only in the owner and keeps the RX-task lifetime
independent. The owner runs outside the timer task and the firmware uses
panic abort if stop or barrier setup fails.
[ESP-IDF v6.1.1 timer implementation](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_timer/src/esp_timer.c)
and [timer API](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_timer/include/esp_timer.h).

The retained RMII resources bound the bridge to a single firmware-lifetime
instance. Adding driver recreation or MAC deletion would require a real RX
task shutdown and join. Keep the mediator installed for the lifetime of the MAC.

## Wi-Fi initialization

The svc 0.53.0 constructor leaves `wifi_task_stack_size` zero. In the ESP32
v6.1.1 archives, `pp_create_task` reads that field through
`config_get_wifi_task_stack_size` and passes it to the SDK task-creation
adapter. The generated SDK config enables WPA3 with nano formatting disabled,
so `WIFI_TASK_STACK_SIZE` resolves to 6,656 bytes.

The firmware links `WifiDriver::new`'s `esp_wifi_init` call through
`components/wifi_init`. The C shim constructs the full
`WIFI_INIT_CONFIG_DEFAULT()` initializer and copies the NVS enable flag from
the svc configuration, preserving the bridge's fallback when NVS is
unavailable. For this SDK configuration the other svc fields match the SDK
defaults, including the disabled privacy enhancements and zero random-MAC
reset interval. The shim checks the stack constant at compile time. The svc
driver still owns initialization, event subscription and teardown.

The [upstream fix](https://github.com/esp-rs/esp-idf/pull/610) is merged.
Its monorepo revision brings newer HAL, sys and Ethernet code. Remove the
shim and linker flag when adopting a compatible svc release with the fix.
Wi-Fi startup and association still require WT32-ETH01 hardware validation.

## Wi-Fi dispatch and teardown

The ESP32 SDK ships Wi-Fi as binary archives. The v6.1.1 Wi-Fi submodule pins
`ec08f0cee472c46e9e9c2c5f8180f74653e8c2dd`. Inspection with
`xtensa-esp32-elf-objdump -dr` on `libnet80211.a` and `libpp.a` established
these implementation details for station mode on this target.
[ESP32 Wi-Fi archives](https://github.com/espressif/esp32-wifi-lib/tree/ec08f0cee472c46e9e9c2c5f8180f74653e8c2dd/esp32).

- `ppTask` handles RX and queued configuration and timer work. Normal delivery
  follows `ppRxPkt`, `sta_rx_cb`, `sta_input` and the registered STA function.
  AMPDU reordering reaches `sta_input` from the same task, including timer
  work posted by `ieee80211_timer_process`. A-MSDU subframes can call the STA
  function inline from `ieee80211_decap_amsdu` under `sta_input`. The STA
  callbacks are serialized in the configured modes.
- `esp_wifi_internal_reg_rxcb` submits ioctl 26 using `wifi_set_rxcb_process`.
  From the bridge owner, `ieee80211_ioctl` posts work item 6 to `ppTask` and
  waits on its completion semaphore. `wifi_sta_reg_rxcb` changes the STA
  function on that task. A successful unregister therefore follows the return
  of any outstanding STA callback, including its release of a rejected buffer.
  Calls made from the Wi-Fi task itself follow a direct path.
- The SDK stops the STA through `esp_wifi_stop`. The svc destructor disconnects,
  stops and calls `esp_wifi_deinit`. The latter unregisters STA/AP RX as part
  of deinit. The bridge uses explicit STA unregister while Wi-Fi is initialized
  as its RX shutdown barrier.
  [ESP-IDF v6.1.1 Wi-Fi private API](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_wifi/include/esp_private/wifi.h)
  and [Wi-Fi deinit source](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_wifi/src/wifi_init.c).

The private header documents one RX function per interface. Serialization and
the unregister barrier are implementation details. Recheck the actual archives
when changing SDK, target, Wi-Fi modes or lifecycle APIs.

The svc 0.53.0 `WifiDriver::handle_tx` still passes `len as usize` to
`slice::from_raw_parts` even though the SDK passes a `uint16_t *`. The wrapper can
construct an invalid slice before invoking the supplied callback. Both
`set_callbacks` and `set_nonstatic_callbacks` install that wrapper. Keep the
direct STA RX registration and leave TX-done callbacks unregistered.
[svc 0.53.0 Wi-Fi wrapper](https://github.com/esp-rs/esp-idf-svc/blob/d42a67836f965eb6fd5d2b04785ab294751c6aa7/src/wifi.rs).

## Admission, buffer release and TX

The two RX tasks are serialized per direction and can run concurrently with
each other and with the owner. The owner publishes the live C Ethernet handle
before opening the gates. Acquire admission makes the handle visible, and
guard release clears only ACTIVE. The owner closes both gates and drains
admitted transmitters. Admission cannot account for callbacks that have already
started but have yet to enter a gate. `Running::drop` unregisters Wi-Fi RX,
clears the handle, stops Ethernet and drains its link timer. The fields then
destroy Wi-Fi before Ethernet. Errors in the barriers are fatal under the
target's panic-abort strategy.

- **Ethernet RX.** The RMII task hands off a malloc-allocated buffer and does
  not free it after dispatch. The callback copies the six source MAC bytes
  once, after checking `len >= 12`, and frees the buffer exactly once on
  every path. Lengths outside `uint16_t` are discarded. The Wi-Fi TX header
  promises a synchronous copy. The archive routes `esp_wifi_internal_tx`
  through `ieee80211_output_do` with a null netstack reference and posts a
  driver-owned TX buffer. Wi-Fi TX failures use the existing counters.
- **Wi-Fi RX.** The STA callback receives a readable frame and its `eb`
  token. On the inspected delivery paths the SDK leaves the token with the
  callback. The callback calls `esp_wifi_internal_free_rx_buffer(eb)` exactly
  once, including gate rejections and TX errors, and returns `ESP_OK`. The
  release API calls the SDK's RX recycling path. The callback synchronously
  calls Ethernet TX with the live C handle when admitted.
- **Ethernet TX.** With the optional Ethernet transmit mutex disabled, the
  single-active gate serializes access to shared DMA TX descriptors.
  `emac_esp_dma_transmit_frame` copies into DMA-owned storage before handing
  the descriptors to hardware. The SDK returns `ESP_ERR_NO_MEM` for a frame
  that cannot fit the configured ring or temporarily unavailable descriptors,
  and `esp_eth_transmit` returns `ESP_ERR_INVALID_STATE` for link down. The
  diagnostics use the configured 30 × 1522-byte TX ring and keep the same
  counters and length buckets.
  [ESP-IDF v6.1.1 ESP32 Ethernet DMA](https://github.com/espressif/esp-idf/blob/v6.1.1/components/esp_eth/src/mac/esp_eth_mac_esp_dma.c).

The bridge forms no Rust slice from either RX buffer. Steady-state forwarding
adds no frame copy, allocation, queue, blocking lock or retry loop. Keep the
single-active bit even though the current producers are serialized. Revisit
descriptor synchronization before allowing concurrent Ethernet senders.
The TX counters measure driver outcomes and cannot confirm descriptor
exhaustion or delivery on the wire.

## Historical ESP-IDF v5.3.3 findings

PR #2 replaced the old integer-cast driver pointers that were used to create
exclusive Rust driver references from concurrent callbacks. Wi-Fi previously
held a path to Ethernet while the driver fields were destroyed in the wrong
order. Closing the forwarding gates could protect admitted transmitters but
could finish while a callback paused before admission. Ethernet `stop()`
did not join the RMII task or make input closure replacement safe. The
static mediator bypassed the old three-argument `eth_stack_input` dispatcher
and addressed late dispatch through a freed mediator.

The v5.3.3 RMII teardown with svc 0.50.1 retained MAC, PHY, DMA and the RX
task. The v5.3.3 Wi-Fi archive at
`711813cb0a41dd2459fd1a3a464d0dd78d323a49` was inspected for synchronous
STA RX dispatch and unregister before destruction. The TX wrapper defect
was observed in svc 0.50.1, 0.51.0, 0.52.1, 0.53.0 and monorepo commit
`21825fe8e6e08270e27eacbacca0be38a3b4c164`. The old Ethernet driver also
stopped its periodic link timer without joining it.
[PR #2 safety record](https://github.com/Hjalle7/esp32-wifi-bridge/blob/97ec056/SAFETY.md)
and [v5.3.3 Wi-Fi archives](https://github.com/espressif/esp32-wifi-lib/tree/711813cb0a41dd2459fd1a3a464d0dd78d323a49/esp32).

The 2026-11-18 PR #2 follow-up used ESP Rust 1.85.0.0, ESP-IDF v5.3.3 and
dummy credentials. Formatting, warnings-as-errors Clippy, 11 standalone host
tests and a locked release build passed. Its 765,584-byte application image
left 282,992 bytes in a 1 MiB factory partition. The later v5.3.3 bench build
at `844f4c5` is recorded in the README. Both builds preceded the TX diagnostics.

## Regression checks and hardware limits

Run the host suites independently of the ESP binary.

```sh
rustc +stable --edition=2021 --test tests/host/callback_gate.rs -o /tmp/bridge-callback-tests
/tmp/bridge-callback-tests
rustc +stable --edition=2021 --test tests/host/ethernet_rx.rs -o /tmp/bridge-ethernet-tests
/tmp/bridge-ethernet-tests
rustc +stable --edition=2021 --test tests/host/tx_diagnostics.rs -o /tmp/bridge-tx-tests
/tmp/bridge-tx-tests
```

These cover admission and draining, static mediator dispatch after modeled
driver removal, low-level hooks, MAC lookup failure and TX counter
classification. The host suites use stubs for driver interfaces. Separately
check formatting, Clippy, locked release cross-compilation and partition fit
with dummy credentials.

WT32-ETH01 hardware validation is still required for MAC cloning,
DHCP/ARP and bidirectional forwarding, sustained traffic and Ethernet TX
diagnostics, link changes during traffic, and startup and teardown with RX/TX
in flight. The existing application connects once and reports TX outcomes
while running. Startup failures can abort.
