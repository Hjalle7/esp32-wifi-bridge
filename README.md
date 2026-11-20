# esp32-wifi-bridge

Wi-Fi to Ethernet bridge targeting the ESP32.

## WT32-ETH01 V1.4 hardware baseline

The bench board has an ESP32-D0WD-V3 revision 3.1 and 8 MB SPI flash. Factory
boot, CP2102 communication, flash identification and a full factory backup
were confirmed before this test. Running the Rust bridge on the board is pending.

The target is `xtensa-esp32-espidf` with ESP-IDF v5.3.3. The LAN87XX driver
uses PHY address 1, MDC on GPIO23, MDIO on GPIO18 and external 50 MHz RMII
clock input on GPIO0. The driver uses GPIO16 as its PHY reset output, which
enables the board's clock oscillator. RXD0/RXD1/CRS_DV use GPIO25/26/27 and
TXD0/TXD1/TX_EN use GPIO19/22/21. [WT32-ETH01 hardware reference](https://github.com/egnor/wt32-eth01#using-esp-idf-with-ethernet)

### Build and check the flash images

Use a shell with the project's ESP Rust toolchain, `ldproxy` and ESP-IDF build
prerequisites available. Run from the repository root. For a build-only check:

```sh
WIFI_SSID=bridge-validation WIFI_PASS=validation-only cargo build --release --locked
```

For association testing, rebuild locally with a dedicated 2.4 GHz WPA2-Personal
test AP's credentials. In Bash on macOS, read them without putting them in shell
history. The resulting ELF and application image contain these values. Keep
`target/`, `.embuild/`, generated SDK configuration and the backup private.

```bash
read -r -p 'Test SSID: ' WIFI_SSID
read -r -s -p 'Test passphrase: ' WIFI_PASS
echo
WIFI_SSID="$WIFI_SSID" WIFI_PASS="$WIFI_PASS" cargo build --release --locked
unset WIFI_SSID WIFI_PASS
```

The configured `cargo run --release` runner is `espflash flash --monitor`.
For this CP2102 procedure, use the release ELF and the bootloader and partition
table that `esp-idf-sys` places alongside it. With the Python 3.13.17
environment containing esptool 5.6.0 active, run:

```sh
python3 -m esptool version
out=target/xtensa-esp32-espidf/release
python3 -m esptool --chip esp32 elf2image --flash-mode dio --flash-freq 40m \
  --flash-size 2MB --min-rev-full 0 --max-rev-full 399 --elf-sha256-offset 0xb0 \
  -o "$out/bridge.bin" "$out/esp32-wifi-bridge"
python3 -m esptool --chip esp32 image-info "$out/bridge.bin"
pt=.embuild/espressif/esp-idf/v5.3.3/components/partition_table
python3 "$pt/gen_esp32part.py" "$out/partition-table.bin"
python3 "$pt/check_sizes.py" partition --type app --subtype factory \
  "$out/partition-table.bin" "$out/bridge.bin"
wc -c "$out/bridge.bin"
```

For an activated external IDF checkout, use its `components/partition_table`
directory for `pt`. Check the image digest, revision range and the decoded
table before flashing. Stop if a command fails or the output differs from this
layout. The single factory app ends at 0x110000 (exclusive), within the SDK's
2 MB header limit and the physical 8 MB flash. The SDK may report the larger
detected size and use the header size.

The Linux baseline with ESP Rust 1.85.0.0, ESP-IDF v5.3.3 and the dummy
credentials above produced a **774,256-byte** application with **274,320 bytes
free**. Recheck the size after building with the bench credentials.

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

CI runs formatting, Clippy, 8 callback-gate tests, 3 mediator ABI-stub tests
and a locked release build with dummy credentials. The UART sequence and
network checks above are expected observations pending a board run. Startup
failures can panic, Wi-Fi connects once, and transmit errors are silent.
Sustained traffic, link-loss recovery and watchdog behavior remain unverified.

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
