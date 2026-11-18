//! Static RMII receive dispatch, independent of the installed Ethernet driver.

use core::{ffi::c_void, ptr};

use esp_idf_svc::sys;

// In ESP-IDF v5.3.3 the ESP32 MAC uses only stack_input and the low-level
// init/deinit notifications. esp-idf-svc installs no low-level user hooks.
// The PHY retains the driver's original mediator for link/SMI operations.
static INPUT: sys::esp_eth_mediator_t = sys::esp_eth_mediator_t {
    phy_reg_read: None,
    phy_reg_write: None,
    stack_input: Some(crate::bridge::forward_to_wifi),
    on_state_changed: Some(low_level_state_changed),
};

const unsafe extern "C" fn low_level_state_changed(
    _mediator: *mut sys::esp_eth_mediator_t,
    _state: sys::esp_eth_state_t,
    _args: *mut c_void,
) -> sys::esp_err_t {
    sys::ESP_OK
}

/// Install once, before the first Ethernet start.
///
/// # Safety
/// `handle` must be a live, initialized, never-started ESP32 RMII driver with
/// no low-level init/deinit user hooks. Do not delete its MAC while RX can run.
///
/// # Panics
/// Panics if the MAC has no `set_mediator` method (not an ESP32 RMII MAC).
pub unsafe fn install(handle: sys::esp_eth_handle_t) -> sys::esp_err_t {
    let mut mac = ptr::null_mut();
    // SAFETY: the caller owns a live driver; the API writes its MAC pointer.
    let result = unsafe { sys::esp_eth_get_mac_instance(handle, &mut mac) };
    if result != sys::ESP_OK {
        return result;
    }

    // SAFETY: the ESP32 MAC's set_mediator stores this pointer without writing
    // through it. INPUT and its functions live forever. No RX has started, so
    // replacing the pointer cannot race dispatch. Crucially, RX never enters
    // eth_stack_input through a driver-owned (and subsequently freed) mediator.
    unsafe { (*mac).set_mediator.unwrap()(mac, ptr::addr_of!(INPUT).cast_mut()) }
}
