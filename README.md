# esp32-wifi-bridge

Wi-Fi to Ethernet bridge targeting the ESP32.

## WT32-ETH01 V1.4 hardware baseline

The bench board has an ESP32-D0WD-V3 revision 3.1 and 8 MB SPI flash. Factory
boot, CP2102 communication, flash identification and a full factory backup
were confirmed before this test. Running the Rust bridge on the board is pending.

The target is `xtensa-esp32-espidf` with ESP-IDF v6.1.1 and ESP Rust 1.97.0.0.
The build pins Espressif's LAN87XX v1.1.0 component by Git commit. The driver
uses PHY address 1, MDC on GPIO23, MDIO on GPIO18 and external 50 MHz RMII
clock input on GPIO0. The driver uses GPIO16 as its PHY reset output, which
enables the board's clock oscillator. RXD0/RXD1/CRS_DV use GPIO25/26/27 and
TXD0/TXD1/TX_EN use GPIO19/22/21. [WT32-ETH01 hardware reference](https://github.com/egnor/wt32-eth01#using-esp-idf-with-ethernet)

### Build and check the flash images

Install ESP Rust 1.97.0.0 as `esp` (for example, with
`espup install --toolchain-version 1.97.0.0 --targets esp32`) and make its
environment, `ldproxy`, and the ESP-IDF build prerequisites available.
Cargo selects the v6.1.1 tag when it manages the SDK. If you activate an
external SDK checkout, verify `idf.py --version` reports v6.1.1. In Codex,
follow the activation and Python checks in [AGENTS.md](AGENTS.md).

Run the build-only check from the repository root with dummy credentials.

```sh
WIFI_SSID=bridge-validation WIFI_PASS=validation-only cargo build --release --locked
```

For association testing, rebuild locally with a dedicated 2.4 GHz WPA2-Personal
test AP's credentials. In zsh on macOS, enter them at the prompts to keep them
out of shell history. The passphrase prompt hides what you type. The resulting
ELF and application image contain these values. Keep
`target/`, `.embuild/`, generated SDK configuration and the backup private.

```zsh
if IFS= read -r 'WIFI_SSID?Test SSID: ' &&
   IFS= read -rs 'WIFI_PASS?Test passphrase: '; then
  printf '\n'
  WIFI_SSID="$WIFI_SSID" WIFI_PASS="$WIFI_PASS" cargo build --release --locked
fi
unset WIFI_SSID WIFI_PASS
```

The configured `cargo run --release` runner is `espflash flash --monitor`.
For this CP2102 procedure, use the release ELF and the bootloader and partition
table that `esp-idf-sys` places alongside it. With the ESP-IDF Python
environment containing esptool 5.6.0 active, run the image and partition checks.

```sh
python3 -m esptool version
out=${CARGO_TARGET_DIR:-target}/xtensa-esp32-espidf/release
python3 -m esptool --chip esp32 elf2image --flash-mode dio --flash-freq 40m \
  --flash-size 2MB --min-rev-full 0 --max-rev-full 399 --elf-sha256-offset 0xb0 \
  -o "$out/bridge.bin" "$out/esp32-wifi-bridge"
python3 -m esptool --chip esp32 image-info "$out/bridge.bin"
pt=${IDF_PATH:-.embuild/espressif/esp-idf/v6.1.1}/components/partition_table
python3 "$pt/gen_esp32part.py" "$out/partition-table.bin"
python3 "$pt/check_sizes.py" --offset 0x8000 bootloader 0x1000 "$out/bootloader.bin"
python3 "$pt/check_sizes.py" partition --type app --subtype factory \
  "$out/partition-table.bin" "$out/bridge.bin"
wc -c "$out/bridge.bin"
```

Check the image digest, revision range and decoded table before flashing.
Stop if a command fails or the output differs from this
layout. The single factory app ends at 0x110000 (exclusive), within the SDK's
2 MB header limit and the physical 8 MB flash. The SDK may report the larger
detected size and use the header size.

The Linux release build with ESP Rust 1.97.0.0, ESP-IDF v6.1.1 and the dummy
credentials above produced a **963,008-byte** application with **85,568 bytes
free** in the factory partition. The 25,904-byte bootloader leaves 2,768
bytes before the partition table. The historical Linux build at `844f4c5`
used ESP Rust 1.85.0.0 and ESP-IDF v5.3.3. It produced a 774,256-byte
application with 274,320 bytes free and predates the TX diagnostics. Recheck
the image sizes after building with the bench credentials.

| Offset | Contents | Size or limit |
| --- | --- | --- |
| 0x1000 | Second-stage bootloader | Up to 0x7000 bytes before the table |
| 0x8000 | Partition table | 0x1000-byte sector |
| 0x9000 | NVS | 0x6000 bytes |
| 0xf000 | PHY init data | 0x1000 bytes |
| 0x10000 | Factory application | 0x100000 bytes (1 MiB) |

### Flash and monitor on macOS

Use the same CP2102 wiring and stable board supply used for the backup. UART
signals are 3.3 V, TX crosses to RX, and grounds are common. Close any serial
monitor. Hold GPIO0 low while resetting EN or cycling power to enter the ROM
loader. With the full original backup retained, write these three files from
the same build. esptool verifies the writes. A full-chip erase is unnecessary.

```sh
port=/dev/cu.usbserial-0001
python3 -m esptool --chip esp32 --port "$port" --baud 115200 \
  --before no-reset --after no-reset write-flash --flash-size keep \
  0x1000 "$out/bootloader.bin" 0x8000 "$out/partition-table.bin" \
  0x10000 "$out/bridge.bin"
```

After successful verification, release GPIO0, open the monitor and reset EN
or power-cycle for normal boot. Capture UART at 115200 8N1. To exit `screen`,
press Ctrl-A, then K, then Y.

```sh
screen /dev/cu.usbserial-0001 115200
```

### Physical acceptance checks

1. **Learn one Ethernet MAC.** Boot with Ethernet disconnected. After ROM and
   IDF startup, look for `Waiting to sniff client MAC...`. Connect one wired
   test client directly and request a DHCP lease to generate a frame. Compare
   the six bytes in `Sniffed client MAC: xx:xx:xx:xx:xx:xx` with the client's
   Ethernet address. Reset the bridge before substituting a different client.
2. **Confirm Wi-Fi association.** Expect `Setting Ethernet promiscuous...`,
   `Ethernet promiscuous success!` and `Wi-Fi configuration set!` twice.
   IDF driver lines can interleave. The configuration messages precede the
   asynchronous connection. Confirm association in the test AP's station list
   using the **wired client's MAC**, and look for the IDF Wi-Fi connected log.
3. **Confirm DHCP and ARP.** Disable alternate network paths on the wired
   client. Once associated, renew its IPv4 DHCP lease; the initial frame used
   for learning can be lost. Check the client received the test LAN's address,
   mask and gateway, and match the lease to the wired MAC in the DHCP server.
   The bridge has no management IP.
4. **Confirm IPv4 forwarding.** From the wired client, ping the gateway by
   IPv4 address and check its ARP entry. From a second LAN host, ping the
   client's leased IPv4 address. Allow ICMP on the test hosts and disable AP
   client isolation for this test. Record the lease, ARP resolution and packet
   counts for both directions. On macOS, `ipconfig getpacket enX`, `arp -an`
   and `ping -c 5 <IPv4-address>` are useful; select the wired interface.
5. **Recover from a Wi-Fi outage.** Leave Ethernet connected and keep traffic
   running. Disable the test AP briefly, then repeat with an outage long
   enough to observe several attempts. Restore the AP each time and confirm
   Ethernet stays up, the bridge associates with the same cloned MAC without
   rebooting, and DHCP, ARP and bidirectional IPv4 work again. On the QE-4,
   confirm Home Assistant can resume Modbus TCP polling.

CI runs formatting, Clippy, 8 callback-gate tests, 3 mediator ABI-stub tests,
3 TX diagnostics tests, 5 reconnection tests and a locked release build with
dummy credentials.
The release job converts the ELF with the esptool settings above and checks
the factory application and bootloader sizes against the generated layout.

The bridge retries Wi-Fi association after a disconnect, including during
extended AP outages. It waits at least five seconds between connection
requests and lets each accepted association attempt finish. Startup failures
can still abort. Sustained traffic and link-loss recovery need hardware
validation.

### Ethernet and Wi-Fi TX diagnostics

Once the bridge reaches `Running`, the serial monitor shows one `bridge.tx`
line about every second, including idle intervals. This example omits the log
prefix.

```text
eth link=up attempts=814 ok=808 no_mem=6 link_down=0 other=0 gate=0 no_mem_len_buckets=[6, 0, 0] no_mem_len_min_max=Some((64, 1514)) | wifi sta=Connected attempts=263 ok=263 failed=0 gate=0
```

| Field | Meaning |
| --- | --- |
| `attempts`, `ok` | Completed TX API calls and calls returning `ESP_OK`, respectively, for each direction. |
| Ethernet `no_mem`, `link_down`, `other` | Calls returning `ESP_ERR_NO_MEM`, `ESP_ERR_INVALID_STATE` (link down at transmit), or any other error. |
| `no_mem_len_buckets` | Counts of Ethernet `ESP_ERR_NO_MEM` frames in the inclusive size buckets ≤1522, 1523–45660, and >45660 bytes, in that order. The verified TX ring has 30 buffers of 1522 bytes each. |
| `no_mem_len_min_max` | Smallest and largest `ESP_ERR_NO_MEM` frame lengths in bytes since boot. `None` means no such failures observed so far. |
| Wi-Fi `failed` | Calls returning any error from the Wi-Fi TX API. |
| Wi-Fi `sta` | Station state sampled at report time. `Unavailable` means the station stopped or returned a driver or configuration error that suspends retries. |
| `gate` | Frames rejected by that direction's forwarding gate while closed or busy, including Ethernet frames seen during MAC learning in the first Wi-Fi report. These frames do not enter `attempts`. |
| Ethernet `link` | Event-based link state sampled at report time. Speed and duplex are omitted. |

On the WT32-ETH01, use the release build and 115200 8N1 monitor procedure above.
Complete the DHCP and IPv4 checks with one wired client, then capture 30 seconds
of quiet traffic. From a second host on the test LAN, send several short bursts
toward the wired client, for example by running `iperf3 -s` on the wired client
and `iperf3 -c <wired-client-ip> -t 5` on the second host. Leave quiet intervals
between bursts. Repeat while disconnecting and reconnecting Ethernet and mark
the times in the serial capture. Reset the bridge before switching wired clients.

Compare `no_mem / attempts` and the size buckets across quiet, burst and
reconnect intervals. `no_mem` concentrated in the smallest bucket during bursts
is consistent with temporary descriptor exhaustion. Larger frames can require
multiple buffers, and frames over 45660 bytes exceed the ring's configured byte
capacity. `link_down` during a cable pull identifies a different TX result.

Each counter is drained independently, so traffic at a reporting boundary can
land in either adjacent interval. The first interval can include startup gate
rejections. Link events can lag a TX failure or occur between samples. `ESP_OK`
measures driver acceptance. These counters cannot see delivery on the wire,
descriptor occupancy, or drops before the forwarding callbacks. Ethernet RX
lengths that cannot be passed to the Wi-Fi API are outside its TX counters.

## Performance

The original project reported approximately 50 Mbps in each direction. Throughput on the V1.4 bench unit still needs measurement.

### Coding Style

Obey `rustfmt` and Rust 2021 conventions, as well as `clippy` lints.

## Contributing

Pull requests are always welcome.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be licensed under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or any later version.

## Versioning

At the moment, this project does not have a stable versioning scheme.

Changes will be documented in the [Changelog](CHANGELOG.md) on a best-effort basis.

See the [tags](https://github.com/owenthewizard/esp32-wifi-bridge/tags) for available releases.

## Authors

See [the list of contributors](https://github.com/owenthewizard/i3lockr/contributors).

## License

Copyright (C) 2024 Owen Walpole

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <http://www.gnu.org/licenses/>.

## Acknowledgments

[esp-rs](https://github.com/esp-rs/) and [Espressif](https://www.espressif.com/) for their stellar support of Rust.
