//! Exercise forwarding counter classification without the physical drivers.
extern crate self as esp_idf_svc;

#[allow(non_camel_case_types)]
pub mod sys {
    pub type esp_err_t = i32;
    pub const ESP_OK: esp_err_t = 0;
    pub const ESP_ERR_NO_MEM: esp_err_t = 0x101;
    pub const ESP_ERR_INVALID_ARG: esp_err_t = 0x102;
    pub const ESP_ERR_INVALID_STATE: esp_err_t = 0x103;
}

#[path = "../../src/tx_diagnostics.rs"]
mod tx_diagnostics;

use sys::{ESP_ERR_INVALID_ARG, ESP_ERR_INVALID_STATE, ESP_ERR_NO_MEM, ESP_OK};
use tx_diagnostics::{EthernetTxCounters, WifiTxCounters};

#[test]
fn ethernet_classifies_results_and_drains_counts() {
    let tx = EthernetTxCounters::new();
    assert_eq!(tx.no_mem_len_range(), None);

    tx.record(ESP_OK, 64);
    tx.record(ESP_ERR_INVALID_STATE, 64);
    tx.record(ESP_ERR_INVALID_ARG, 0);
    for len in [1522, 1523, 45660, 45661] {
        tx.record(ESP_ERR_NO_MEM, len);
    }
    tx.reject();

    let interval = tx.take_interval();
    assert_eq!(interval.attempts(), 7);
    assert_eq!(interval.ok, 1);
    assert_eq!(interval.no_mem(), 4);
    assert_eq!(interval.link_down, 1);
    assert_eq!(interval.other, 1);
    assert_eq!(interval.gate, 1);
    assert_eq!(interval.no_mem_len_buckets, [1, 2, 1]);
    assert_eq!(tx.no_mem_len_range(), Some((1522, 45661)));

    let next = tx.take_interval();
    assert_eq!(next.attempts(), 0);
    assert_eq!(next.gate, 0);
    assert_eq!(tx.no_mem_len_range(), Some((1522, 45661)));
}

#[test]
fn ethernet_length_range_keeps_both_extremes_since_boot() {
    let tx = EthernetTxCounters::new();
    tx.record(ESP_ERR_NO_MEM, u16::MAX);
    assert_eq!(tx.no_mem_len_range(), Some((u16::MAX, u16::MAX)));
    tx.take_interval();
    tx.record(ESP_ERR_NO_MEM, 0);
    tx.record(ESP_ERR_NO_MEM, 64);
    assert_eq!(tx.no_mem_len_range(), Some((0, u16::MAX)));
    assert_eq!(tx.take_interval().no_mem_len_buckets, [2, 0, 0]);
}

#[test]
fn wifi_counts_api_outcomes_separately_from_gate_rejections() {
    let tx = WifiTxCounters::new();
    tx.record(ESP_OK);
    tx.record(ESP_ERR_NO_MEM);
    tx.record(ESP_ERR_INVALID_STATE);
    tx.reject();

    let interval = tx.take_interval();
    assert_eq!(interval.attempts(), 3);
    assert_eq!(interval.ok, 1);
    assert_eq!(interval.failed, 2);
    assert_eq!(interval.gate, 1);

    let next = tx.take_interval();
    assert_eq!(next.attempts(), 0);
    assert_eq!(next.gate, 0);
}
