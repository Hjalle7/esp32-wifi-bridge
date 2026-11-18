//! Exercise the actual static-mediator installer against a small C ABI stub.
extern crate self as esp_idf_svc;

#[allow(non_camel_case_types)]
pub mod sys {
    use core::ffi::c_void;

    pub type esp_err_t = i32;
    pub type esp_eth_state_t = u32;
    pub type esp_eth_handle_t = *mut c_void;
    pub const ESP_OK: esp_err_t = 0;
    pub const ESP_FAIL: esp_err_t = -1;

    #[repr(C)]
    pub struct esp_eth_mediator_t {
        pub phy_reg_read: Option<unsafe extern "C" fn()>,
        pub phy_reg_write: Option<unsafe extern "C" fn()>,
        pub stack_input: Option<unsafe extern "C" fn(*mut Self, *mut u8, u32) -> esp_err_t>,
        pub on_state_changed:
            Option<unsafe extern "C" fn(*mut Self, esp_eth_state_t, *mut c_void) -> esp_err_t>,
    }

    pub struct esp_eth_mac_t {
        pub set_mediator:
            Option<unsafe extern "C" fn(*mut Self, *mut esp_eth_mediator_t) -> esp_err_t>,
        pub input: *mut esp_eth_mediator_t,
    }

    pub struct Driver {
        pub mac: esp_eth_mac_t,
        pub get_result: esp_err_t,
    }

    pub unsafe fn esp_eth_get_mac_instance(
        handle: esp_eth_handle_t,
        mac: *mut *mut esp_eth_mac_t,
    ) -> esp_err_t {
        let driver = unsafe { &mut *handle.cast::<Driver>() };
        if driver.get_result == ESP_OK {
            unsafe { mac.write(&mut driver.mac) };
        }
        driver.get_result
    }
}

mod bridge {
    use super::sys;
    pub unsafe extern "C" fn forward_to_wifi(
        _mediator: *mut sys::esp_eth_mediator_t,
        buf: *mut u8,
        len: u32,
    ) -> sys::esp_err_t {
        assert_eq!(len, 1);
        // The stub callback consumes a caller-owned byte without a driver.
        unsafe { *buf += 1 };
        sys::ESP_OK
    }
}

#[path = "../../src/ethernet_rx.rs"]
mod ethernet_rx;

unsafe extern "C" fn set_mediator(
    mac: *mut sys::esp_eth_mac_t,
    input: *mut sys::esp_eth_mediator_t,
) -> sys::esp_err_t {
    unsafe { (*mac).input = input };
    sys::ESP_OK
}

fn driver(get_result: sys::esp_err_t) -> Box<sys::Driver> {
    Box::new(sys::Driver {
        mac: sys::esp_eth_mac_t {
            set_mediator: Some(set_mediator),
            input: core::ptr::null_mut(),
        },
        get_result,
    })
}

#[test]
fn rx_dispatch_can_start_after_driver_storage_is_freed() {
    let mut driver = driver(sys::ESP_OK);
    assert_eq!(
        unsafe { ethernet_rx::install((&raw mut *driver).cast()) },
        sys::ESP_OK
    );
    let input = driver.mac.input;
    drop(driver);

    // Model the outstanding RMII task dispatching only after uninstall. This
    // would dereference freed storage with the original driver-owned mediator.
    let mut byte = 0;
    let result = unsafe { (*input).stack_input.unwrap()(input, &mut byte, 1) };
    assert_eq!(result, sys::ESP_OK);
    assert_eq!(byte, 1);
}

#[test]
fn no_user_low_level_hooks_are_needed_during_deinit() {
    let mut driver = driver(sys::ESP_OK);
    assert_eq!(
        unsafe { ethernet_rx::install((&raw mut *driver).cast()) },
        sys::ESP_OK
    );
    let input = driver.mac.input;
    assert_eq!(
        unsafe { (*input).on_state_changed.unwrap()(input, 0, core::ptr::null_mut()) },
        sys::ESP_OK
    );
    assert!(unsafe { (*input).phy_reg_read.is_none() && (*input).phy_reg_write.is_none() });
}

#[test]
fn get_mac_failure_is_propagated_without_installing() {
    let mut driver = driver(sys::ESP_FAIL);
    assert_eq!(
        unsafe { ethernet_rx::install((&raw mut *driver).cast()) },
        sys::ESP_FAIL
    );
    assert!(driver.mac.input.is_null());
}
