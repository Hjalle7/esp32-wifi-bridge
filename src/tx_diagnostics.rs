//! Atomic forwarding counters drained by the main task once per report.
//! Sum completed outcomes to count attempts with one counter increment per call.

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use esp_idf_svc::sys::{esp_err_t, ESP_ERR_INVALID_STATE, ESP_ERR_NO_MEM, ESP_OK};

// Verified ESP-IDF v6.1.1 release sdkconfig, also set in sdkconfig.defaults.
const DMA_BUFFER_BYTES: u16 = 1522;
const DMA_TX_CAPACITY_BYTES: u16 = DMA_BUFFER_BYTES * 30;
// Store the minimum and maximum together so the reporter sees a coherent pair.
const EMPTY_RANGE: u32 = 0xffff_0000;

pub struct EthernetTxCounters {
    ok: AtomicU32,
    link_down: AtomicU32,
    other: AtomicU32,
    gate: AtomicU32,
    no_mem_len_buckets: [AtomicU32; 3],
    no_mem_len_range: AtomicU32,
}

pub struct EthernetInterval {
    pub ok: u32,
    pub link_down: u32,
    pub other: u32,
    pub gate: u32,
    pub no_mem_len_buckets: [u32; 3],
}

impl EthernetInterval {
    pub fn no_mem(&self) -> u32 {
        self.no_mem_len_buckets.iter().sum()
    }

    pub fn attempts(&self) -> u32 {
        self.ok + self.no_mem() + self.link_down + self.other
    }
}

impl EthernetTxCounters {
    pub const fn new() -> Self {
        Self {
            ok: AtomicU32::new(0),
            link_down: AtomicU32::new(0),
            other: AtomicU32::new(0),
            gate: AtomicU32::new(0),
            no_mem_len_buckets: [const { AtomicU32::new(0) }; 3],
            no_mem_len_range: AtomicU32::new(EMPTY_RANGE),
        }
    }

    /// Call only with the Ethernet TX gate held, which serializes range updates.
    pub fn record(&self, result: esp_err_t, len: u16) {
        let count = match result {
            ESP_OK => &self.ok,
            ESP_ERR_INVALID_STATE => &self.link_down,
            ESP_ERR_NO_MEM => {
                let (min, max) = unpack_range(self.no_mem_len_range.load(Relaxed));
                self.no_mem_len_range.store(
                    (u32::from(min.min(len)) << 16) | u32::from(max.max(len)),
                    Relaxed,
                );
                let bucket = if len <= DMA_BUFFER_BYTES {
                    0
                } else if len <= DMA_TX_CAPACITY_BYTES {
                    1
                } else {
                    2
                };
                &self.no_mem_len_buckets[bucket]
            }
            _ => &self.other,
        };
        count.fetch_add(1, Relaxed);
    }

    pub fn reject(&self) {
        self.gate.fetch_add(1, Relaxed);
    }

    pub fn take_interval(&self) -> EthernetInterval {
        EthernetInterval {
            ok: self.ok.swap(0, Relaxed),
            link_down: self.link_down.swap(0, Relaxed),
            other: self.other.swap(0, Relaxed),
            gate: self.gate.swap(0, Relaxed),
            no_mem_len_buckets: core::array::from_fn(|i| {
                self.no_mem_len_buckets[i].swap(0, Relaxed)
            }),
        }
    }

    pub fn no_mem_len_range(&self) -> Option<(u16, u16)> {
        let range = self.no_mem_len_range.load(Relaxed);
        (range != EMPTY_RANGE).then(|| unpack_range(range))
    }
}

const fn unpack_range(range: u32) -> (u16, u16) {
    let [min_hi, min_lo, max_hi, max_lo] = range.to_be_bytes();
    (
        u16::from_be_bytes([min_hi, min_lo]),
        u16::from_be_bytes([max_hi, max_lo]),
    )
}

pub struct WifiTxCounters {
    ok: AtomicU32,
    failed: AtomicU32,
    gate: AtomicU32,
}

pub struct WifiInterval {
    pub ok: u32,
    pub failed: u32,
    pub gate: u32,
}

impl WifiInterval {
    pub const fn attempts(&self) -> u32 {
        self.ok + self.failed
    }
}

impl WifiTxCounters {
    pub const fn new() -> Self {
        Self {
            ok: AtomicU32::new(0),
            failed: AtomicU32::new(0),
            gate: AtomicU32::new(0),
        }
    }

    pub fn record(&self, result: esp_err_t) {
        let count = if result == ESP_OK {
            &self.ok
        } else {
            &self.failed
        };
        count.fetch_add(1, Relaxed);
    }

    pub fn reject(&self) {
        self.gate.fetch_add(1, Relaxed);
    }

    pub fn take_interval(&self) -> WifiInterval {
        WifiInterval {
            ok: self.ok.swap(0, Relaxed),
            failed: self.failed.swap(0, Relaxed),
            gate: self.gate.swap(0, Relaxed),
        }
    }
}
