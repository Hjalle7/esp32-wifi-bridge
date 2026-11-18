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

The review of PR #2 at `1b9859415498feb9591efc5e5e9d4eb0bc15e5ad`
found that its gate argument protected transmitters, but did **not** establish
RX callback quiescence. A callback can start and pause before `try_enter`.
Closing/draining the gate can finish while that callback is still running.
Moving the admission instruction earlier, or counting admitted callbacks,
would not remove the SDK dispatch window before the first Rust instruction.

### Ethernet: the driver-owned dispatcher was unsafe

The following are implementation facts for ESP-IDF v5.3.3, not assumptions
about generic Ethernet drivers:

- `esp_eth_update_input_path` assigns two ordinary fields (`priv` and
  `stack_input`). It provides no synchronization or callback join.
- The ESP32 RMII MAC creates exactly one `emac_rx` task. That task invokes
  `emac->eth->stack_input` synchronously for each received malloc buffer, then
  accesses the MAC's DMA state again. Same-direction callbacks cannot overlap.
- The default `eth_stack_input` dispatcher dereferences the driver-owned
  mediator to read the registered input function and context **before** it
  enters the application callback. There is no receive-side reference increment.
- `esp_eth_stop` stops the link timer, sets link down (disables the interrupt
  and stops hardware), and posts a stop event. It neither joins nor deletes RX.
  A pending notification or a task already processing RX can outlive stop.
- `esp_eth_driver_uninstall` checks stopped/refcount state, deletes the link
  timer, deinitializes PHY/MAC, and frees the driver, including its mediator.
  MAC `deinit` stops hardware and emits a low-level deinit notification; it
  does not join RX. The RX task is deleted only by MAC `del`.
- Locked `esp-idf-svc 0.50.1` `EthDriver::clear_all` stops and uninstalls;
  `RmiiEth` is empty and neither it nor `EthDriver::drop` calls MAC/PHY `del`.
  Consequently the MAC, DMA, PHY and RX task are retained after uninstall.
  Claiming that this destructor deletes/joins the RMII task was incorrect.

A task paused before the default dispatcher reads the input function could
therefore read a freed mediator. Static Rust callbacks and closed gates do not
fix that C use-after-free.

`src/ethernet_rx.rs` installs an immutable **static MAC mediator** using the
public `esp_eth_get_mac_instance` and MAC `set_mediator` APIs, once, after
initialization and before the first start. The RMII `set_mediator` only stores
the pointer. RX now dispatches directly through static storage and never enters
`eth_stack_input`. Its callback needs no handle or driver-owned context.
The PHY keeps its original mediator for SMI/link operations. Source inspection
shows the ESP32 MAC uses only `stack_input` and low-level init/deinit notices;
those notices had no user hooks in the svc constructor and remain no-ops.
This is specifically an ESP32 RMII adapter, not a generic replacement mediator.

A late Ethernet callback may still copy the first source MAC into the static
`OnceCell` and release its owned malloc buffer. Neither operation touches an
installed Ethernet driver or Wi-Fi state. After gate closure it cannot transmit.
MAC formatting/logging now runs in the owner after learning, outside RX.
The MAC/DMA/task remain valid because the locked svc destructor retains them.
No assumption that `vTaskDelete` cooperatively finishes Rust code is used:
FreeRTOS explicitly warns against deleting tasks with locks or allocations
outstanding, and its cross-core deletion requests a yield, not a callback join.

**Remaining limitation:** the pre-existing MAC/PHY/DMA/RX-task resource leak is
not repaired here. The singleton prevents another bridge instance and this
program runs it forever. Do not add MAC deletion, recreate drivers, replace the
mediator after start, or change the dependency's destructor without implementing
an actual RMII RX-task shutdown/join first. The static adapter adds no per-packet
allocation, copy, lock, queue or polling. It does not introduce a new leak.

### Wi-Fi: serialized task dispatch, with explicit unregister before destruction

The private header documents one registered RX function per interface; that
alone is **not** a serialization or join guarantee. The public `esp_wifi_stop`
contract stops STA and frees its control block. `esp_wifi_deinit` frees init
resources and stops the Wi-Fi task. The open `wifi_init.c` unregisters STA/AP RX
before calling `esp_wifi_deinit_internal`. None of these header comments alone
proves what happens to an already-entered callback.

The ESP32 implementation is supplied as binary archives. To check the actual
pinned implementation, inspect the v5.3.3 Wi-Fi submodule
[`711813cb0a41dd2459fd1a3a464d0dd78d323a49`](https://github.com/espressif/esp32-wifi-lib/tree/711813cb0a41dd2459fd1a3a464d0dd78d323a49/esp32)
using `xtensa-esp32-elf-objdump -dr` on `libnet80211.a` and `libpp.a`:

- `ppTask` processes RX (`ppRxPkt`) and queued configuration/timer work in a
  single task. `sta_rx_cb -> sta_input` calls `sta_rxcb` synchronously.
  AMPDU delivery (`ampdu_dispatch -> sta_input`) also belongs to that task;
  `ieee80211_timer_process` posts timer work to it, rather than delivering
  packets from the timer task. These STA callbacks are serialized, including
  delivery of reordered frames. They are not application event-loop callbacks.
- `esp_wifi_internal_reg_rxcb` constructs ioctl operation 26 with
  `wifi_set_rxcb_process`. From an external owner task, `ieee80211_ioctl`
  posts operation 6 to `ppTask` and waits on its completion semaphore.
  `wifi_set_rxcb_process -> wifi_sta_reg_rxcb` updates `sta_rxcb` there.
  The Wi-Fi task must finish its current callback, including rejected-frame
  freeing, before it can perform the unregister and acknowledge completion.
  Calls from the Wi-Fi task take a direct path; our owner is not that task.
- `esp_wifi_stop` likewise sends synchronous ioctl work to the Wi-Fi task.
  `esp_wifi_deinit_internal` and `ppTask` implement task shutdown, including
  the task-deletion semaphore. We do not substitute that for explicit RX
  unregister or claim that the closed gate has drained driver buffer releases.

`Running::drop` closes both gates, waits for admitted transmitters, then
explicitly unregisters STA RX while Wi-Fi is still initialized. A successful
unregister is the complete Wi-Fi callback barrier, including callbacks paused
before gate admission. Failure is fatal, following the existing destructor
policy; destruction does not continue on a failed barrier. Then the Ethernet
handle is cleared and fields drop Wi-Fi first, Ethernet second. The Wi-Fi
wrapper's normal stop/deinit/unregister still runs. Setup failures also use this
owner with gates closed. Earlier states have only the static Ethernet path.

These are version/target-specific implementation observations, **not** a
portable documented guarantee of the private RX API. Recheck the archives when
changing ESP-IDF, target, RX modes or registration/lifecycle APIs. Promiscuous,
CSI, AP and ESP-NOW callback contracts are not used as evidence for station RX.

### Admission and packet loss

Both RX producers are serialized per direction in this pinned configuration.
They can run concurrently with **each other** and with the teardown owner.
There is consequently no normal-operation packet loss from the gate's busy
case: the prior guard is released before the same task invokes its next
callback. The bridge's callbacks do not recursively invoke RX. The opposite
directions have independent gates and cannot reject each other.

Keep the atomic gates for startup publication and close-versus-transmit
synchronization. Serialization by a producer task does not synchronize it with
another task destroying its destination driver. A successful acquire observes
the release publishing the initialized Ethernet handle. Guard release clears
only ACTIVE, never reopening a closed gate; the owner's acquire of idle drains
admitted transmitters. No callback reconstructs a Rust driver reference.

The single-active bit is defensive serialization, not a claimed SDK join.
If RX becomes concurrent in a future SDK, a counted admission gate could avoid
busy drops for a thread-safe destination. It is not safe to simply make that
change here: RMII DMA transmit mutates shared descriptors, and ESP-IDF's optional
`CONFIG_ETH_TRANSMIT_MUTEX` defaults off. A queue/worker or enabling its blocking
mutex would add cost and complexity without improving the current serialized
RX path. No such change is warranted for v5.3.3.

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
or retry loop to steady-state callbacks. MAC learning happens once; log
formatting runs in the owner. Only owner teardown waits (gate draining and
synchronous Wi-Fi unregister).

Relevant inspected source contracts:

- [ESP-IDF v5.3.3 Wi-Fi private API](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_wifi/include/esp_private/wifi.h):
  `wifi_rxcb_t`, `esp_wifi_internal_tx`, `esp_wifi_internal_free_rx_buffer`,
  `wifi_tx_done_cb_t`.
- [ESP-IDF v5.3.3 Ethernet driver](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_eth/src/esp_eth.c):
  `eth_stack_input`, input-path registration, transmit, stop, uninstall, and
  public MAC-instance access.
- [ESP32 RMII MAC](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_eth/src/mac/esp_eth_mac_esp.c)
  and [DMA](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_eth/src/mac/esp_eth_mac_esp_dma.c):
  single RX task and synchronous transmit copying.
- [esp-idf-svc 0.50.1 Wi-Fi](https://github.com/esp-rs/esp-idf-svc/blob/v0.50.1/src/wifi.rs)
  and [Ethernet](https://github.com/esp-rs/esp-idf-svc/blob/v0.50.1/src/eth.rs):
  callback registration, RX ownership/freeing, `RmiiEth`, and driver destructors.
- [Wi-Fi lifecycle contracts](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_wifi/include/esp_wifi.h)
  and [deinit source](https://github.com/espressif/esp-idf/blob/v5.3.3/components/esp_wifi/src/wifi_init.c).
- [FreeRTOS task deletion](https://github.com/espressif/esp-idf/blob/v5.3.3/docs/en/api-reference/system/freertos_idf.rst)
  and [kernel implementation](https://github.com/espressif/esp-idf/blob/v5.3.3/components/freertos/FreeRTOS-Kernel/tasks.c).

## Regression checks and hardware limits

The admission/draining primitive has host tests, including concurrent senders,
close during an active callback, a close/admission race, a callback paused
before admission, and independence of opposite directions. A separate ABI-stub
test exercises the actual static-mediator installer, including dispatch after
driver storage is freed, low-level hooks, and MAC-lookup failure. Run independently
of the ESP binary (`harness = false`) and its target build configuration:

```sh
rustc +stable --edition=2021 --test tests/host/callback_gate.rs -o /tmp/bridge-callback-tests
/tmp/bridge-callback-tests
rustc +stable --edition=2021 --test tests/host/ethernet_rx.rs -o /tmp/bridge-ethernet-tests
/tmp/bridge-ethernet-tests
```

These tests exercise the actual gate and mediator installer with a stub C ABI,
not ESP-IDF or physical drivers. Separately run formatting, Clippy, and release
build commands with dummy `WIFI_SSID` / `WIFI_PASS`. Compilation does not prove
runtime memory safety. WT32-ETH01 checks still need real hardware: MAC cloning,
bidirectional load, DHCP/ARP pass-through, link loss during traffic, startup
failures, and dropping the bridge with RX/TX in flight. Confirm the release image
fits the configured partition on a 4 MB device; never commit generated firmware.

Local follow-up verification (2026-11-18): ESP Rust `1.85.0.0`, ESP-IDF
`v5.3.3`, dummy credentials. Formatting, standalone host rustfmt, the required
all-target/all-feature Clippy command and `cargo build --release --locked` passed.
All 8 gate tests and 3 static-mediator ABI-stub tests passed. The generated
application image is 765,584 bytes, leaving 282,992 bytes in the existing
1 MiB factory partition. The unchanged default image header specifies 2 MiB
flash, within this board's 4 MB capacity. No hardware checks were performed.
