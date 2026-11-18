//! Wi-Fi to Ethernet Bridge State Machine

extern crate alloc;
use alloc::{format, string::String};
use core::{
    ffi::c_void,
    sync::atomic::{AtomicPtr, Ordering},
};

use esp_idf_svc::{
    eth::{EthDriver, RmiiClockConfig, RmiiEth, RmiiEthChipset},
    eventloop::EspSystemEventLoop,
    hal::{delay::FreeRtos, gpio, modem::Modem, prelude::Peripherals},
    handle::RawHandle,
    nvs::EspDefaultNvsPartition,
    sys::{self, esp, esp_err_t, ESP_OK},
    wifi::{AuthMethod, ClientConfiguration, Configuration, WifiDeviceId, WifiDriver},
};

use once_cell::sync::OnceCell;

use crate::callback_gate::CallbackGate;

// ESP-IDF Wi-Fi and Peripherals::take() are singletons. Callbacks use only these
// static gates and the C Ethernet handle, never a pointer/reference to a Rust driver.
static TO_WIFI: CallbackGate = CallbackGate::new();
static TO_ETH: CallbackGate = CallbackGate::new();
static ETH_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static CLIENT_MAC: OnceCell<[u8; 6]> = OnceCell::new();

const SSID: &str = env!("WIFI_SSID");
const PASS: &str = env!("WIFI_PASS");
const AUTH: AuthMethod = AuthMethod::WPA2Personal;

/// Wi-Fi to Ethernet Bridge State Machine
pub struct Bridge<S> {
    state: S,
}

/// Idle State
///
/// In this state, `peripherals`, `sysloop`, and `nvs` are initialized,
/// but no action is being performed.
pub struct Idle {
    peripherals: Peripherals,
    sysloop: EspSystemEventLoop,
    nvs: Option<EspDefaultNvsPartition>,
}

/// Ethernet Ready State
///
/// In this state, [Ethernet](esp_idf_svc::eth::EthDriver) is ready to be transitioned into the
/// [`Running`] state. Additionally, `nvs`, `modem`, and `client_mac` have been initialized and are
/// ready to be used to bring Wi-Fi up.
/// Notably, `client_mac` is sniffed from the source MAC of the first Ethernet frame we catch.
/// At some point after we have sniffed `client_mac` (not necessarily immediately), we stop
/// sniffing future frames.
pub struct EthReady {
    modem: Modem,
    sysloop: EspSystemEventLoop,
    nvs: Option<EspDefaultNvsPartition>,
    eth: EthDriver<'static, RmiiEth>,
    client_mac: [u8; 6],
}

/// Wi-Fi Ready State
///
/// In this state, Wi-Fi is ready to be transitioned into the [`Running`] state.
/// Notably, the Wi-Fi `Sta` MAC has been set to `client_mac`.
pub struct WifiReady {
    eth: EthDriver<'static, RmiiEth>,
    wifi: WifiDriver<'static>,
}

/// Running State
///
/// Owns both drivers. Drop closes and drains forwarding before destroying Wi-Fi,
/// then Ethernet (field order). See `SAFETY.md` for the callback/ESP-IDF contracts.
pub struct Running {
    wifi: WifiDriver<'static>,
    eth: EthDriver<'static, RmiiEth>,
}

impl Drop for Running {
    fn drop(&mut self) {
        TO_WIFI.close();
        TO_ETH.close();
        // Only the owner waits. Packet callbacks never wait for a gate or owner.
        while !TO_WIFI.is_idle() || !TO_ETH.is_idle() {
            FreeRtos::delay_ms(1);
        }
        ETH_HANDLE.store(core::ptr::null_mut(), Ordering::Relaxed);
        // Field destructors now deinitialize Wi-Fi (unregistering RX), then
        // stop/uninstall Ethernet. Neither callback has a Rust-owned environment.
    }
}

impl Bridge<Idle> {
    /// Construct a `Bridge` in the `Idle` state.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use sm::*;
    ///
    /// let idle_bridge = Bridge::new();
    /// ```
    ///
    /// # Panics
    ///
    /// This function calls `take()` on
    /// [`Peripherals`], [`EspSystemEventLoop`], and [`EspDefaultNvsPartition`], and will panic if
    /// any of them return `Err`. Therefore, only one instance of `Bridge` should exist at any given
    /// time, and you shouldn't be using them elsewhere.
    pub fn new() -> Self {
        let peripherals = Peripherals::take().expect("Failed to take peripherals!");
        let sysloop = EspSystemEventLoop::take().expect("Failed to take sysloop!");
        let nvs = EspDefaultNvsPartition::take().ok();

        Self {
            state: Idle {
                peripherals,
                sysloop,
                nvs,
            },
        }
    }
}

/// Transition from [`Idle`] to [`EthReady`].
impl From<Bridge<Idle>> for Bridge<EthReady> {
    fn from(val: Bridge<Idle>) -> Self {
        let pins = val.state.peripherals.pins;
        let mut eth = EthDriver::new_rmii(
            val.state.peripherals.mac,
            pins.gpio25, // RMII RDX0
            pins.gpio26, // RMII RDX1
            pins.gpio27, // RMII CRS DV
            pins.gpio23, // WT32-ETH01 SMI MDC
            pins.gpio22, // EMII TXD1
            pins.gpio21, // RMII TX EN
            pins.gpio19, // RMII TXD0
            pins.gpio18, // WT32-ETH01 SMI MDIO
            RmiiClockConfig::<gpio::Gpio0, gpio::Gpio16, gpio::Gpio17>::Input(
                pins.gpio0, // WT32-ETH01 external clock
            ),
            Some(pins.gpio16), // WT32-ETH01 PHY reset
            RmiiEthChipset::LAN87XX,
            Some(1), // WT32-ETH01 PHY address
            val.state.sysloop.clone(),
        )
        .expect("Failed to init EthDriver!");

        // Register exactly once, before starting RX. Replacing a callback after
        // stop() alone does not establish that its RX task has returned. A static
        // function also avoids a closure environment being freed during teardown.
        // SAFETY: eth is live and stopped, and the callback needs no context.
        // It consumes/frees every RX buffer according to the input-path contract.
        esp!(unsafe {
            sys::esp_eth_update_input_path(
                eth.handle(),
                Some(forward_to_wifi),
                core::ptr::null_mut(),
            )
        })
        .expect("Failed to set Ethernet RX callback!");

        log::warn!("Waiting to sniff client MAC...");
        eth.start().expect("Failed to start Ethernet!");
        let client_mac = *CLIENT_MAC.wait();

        eth.stop()
            .expect("Failed to stop Ethernet after MAC sniffing!");

        log::warn!("Setting Ethernet promiscuous...");
        eth.set_promiscuous(true)
            .expect("Failed to set Ethernet promiscuous!");
        log::warn!("Ethernet promiscuous success!");

        Self {
            state: EthReady {
                modem: val.state.peripherals.modem,
                sysloop: val.state.sysloop,
                nvs: val.state.nvs,
                eth,
                client_mac,
            },
        }
    }
}

/// Transition from [`EthReady`] to [`WifiReady`].
// Retain fatal credential validation, matching the Running transition below.
#[allow(clippy::fallible_impl_from)]
impl From<Bridge<EthReady>> for Bridge<WifiReady> {
    fn from(val: Bridge<EthReady>) -> Self {
        let mut wifi = WifiDriver::new(val.state.modem, val.state.sysloop.clone(), val.state.nvs)
            .expect("Failed to init WifiDriver!");

        let wifi_config = Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            auth_method: AUTH,
            password: PASS.try_into().unwrap(),
            ..Default::default()
        });

        wifi.set_configuration(&wifi_config)
            .expect("Failed to set Wi-Fi configuration!");
        log::warn!("Wi-Fi configuration set!");

        wifi.set_mac(WifiDeviceId::Sta, val.state.client_mac)
            .expect("Failed to set Wi-Fi MAC!");

        Self {
            state: WifiReady {
                eth: val.state.eth,
                wifi,
            },
        }
    }
}

/// Transition from [`WifiReady`] to [`Running`].
// Retain the existing fatal initialization policy for state transitions.
#[allow(clippy::fallible_impl_from)]
impl From<Bridge<WifiReady>> for Bridge<Running> {
    fn from(val: Bridge<WifiReady>) -> Self {
        // Establish teardown ownership before registration or any fallible start.
        // Gates stay closed throughout fallible driver initialization.
        let mut running = Running {
            wifi: val.state.wifi,
            eth: val.state.eth,
        };

        let wifi_config = Configuration::Client(ClientConfiguration {
            ssid: SSID.try_into().unwrap(),
            auth_method: AUTH,
            password: PASS.try_into().unwrap(),
            ..Default::default()
        });

        running
            .wifi
            .set_configuration(&wifi_config)
            .expect("Failed to set Wi-Fi configuration!");
        log::warn!("Wi-Fi configuration set!");

        ETH_HANDLE.store(running.eth.handle(), Ordering::Relaxed);

        // SAFETY: Client-only Wi-Fi is initialized and stopped. The callback is a
        // static function with no borrowed environment. Running owns teardown.
        // Deliberately register RX only: esp-idf-svc 0.50.1's TX wrapper creates
        // an invalid slice even with an empty callback; we never install it.
        esp!(unsafe {
            sys::esp_wifi_internal_reg_rxcb(WifiDeviceId::Sta.into(), Some(forward_to_eth))
        })
        .expect("Failed to set Wi-Fi RX callback!");

        running.wifi.start().expect("Failed to start Wi-Fi!");
        running.eth.start().expect("Failed to start Ethernet!");
        running.wifi.connect().expect("Failed to connect to Wi-Fi!");

        // Release/acquire admission publishes the handle only after both drivers
        // are owned and started. Neither callback accesses the Rust driver values.
        TO_WIFI.open();
        TO_ETH.open();

        Self { state: running }
    }
}

unsafe extern "C" fn forward_to_wifi(
    _handle: sys::esp_eth_handle_t,
    buf: *mut u8,
    len: u32,
    _context: *mut c_void,
) -> esp_err_t {
    if CLIENT_MAC.get().is_none() && len >= 12 {
        let mut mac = [0; 6];
        // SAFETY: ESP-IDF supplies a readable RX buffer of len bytes. The checked
        // source-MAC range is within it and the local destination cannot overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(buf.add(6), mac.as_mut_ptr(), mac.len());
        }
        if CLIENT_MAC.set(mac).is_ok() {
            log::warn!("Sniffed client MAC: {}", mac2str(mac));
        }
    }

    let guard = TO_WIFI.try_enter();
    if guard.is_some() {
        if let Ok(len) = u16::try_from(len) {
            // SAFETY: the gate keeps Wi-Fi initialized until return and
            // serializes transmitters. ESP-IDF v5.3.3 copies the valid Ethernet
            // buffer synchronously, without retaining it. Not-ready/disconnected
            // states are handled by the transmit API's error return.
            unsafe {
                sys::esp_wifi_internal_tx(WifiDeviceId::Sta.into(), buf.cast(), len);
            }
        }
    }
    // SAFETY: Ethernet transfers ownership of this malloc-allocated RX buffer to
    // the input callback. Free exactly once, also on closed/failed transmit.
    unsafe {
        sys::free(buf.cast());
    }
    ESP_OK
}

unsafe extern "C" fn forward_to_eth(buf: *mut c_void, len: u16, eb: *mut c_void) -> esp_err_t {
    let guard = TO_ETH.try_enter();
    if guard.is_some() {
        // SAFETY: ESP-IDF supplies a valid RX buffer of len bytes. Admission
        // publishes a live C handle and prevents its destruction until return.
        // Only one sender enters this direction; RMII transmit copies into DMA
        // buffers before returning and does not retain buf. No Rust driver
        // reference (mutable or shared) is constructed.
        unsafe {
            sys::esp_eth_transmit(ETH_HANDLE.load(Ordering::Relaxed), buf, usize::from(len));
        }
    }
    // SAFETY: RX ownership transfers to this callback. Free eb exactly once,
    // after synchronous transmit (also when closed or transmit fails). Returning
    // ESP_OK tells ESP-IDF the frame was consumed. No slice or TX callback needed.
    unsafe {
        sys::esp_wifi_internal_free_rx_buffer(eb);
    }
    ESP_OK
}

/// Format MAC bytes as a hex string.
///
/// E.g. `02:aa:bb:cc:12:34`
#[inline]
fn mac2str(mac: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
