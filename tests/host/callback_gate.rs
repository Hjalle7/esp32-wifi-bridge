#[path = "../../src/callback_gate.rs"]
mod callback_gate;

use callback_gate::CallbackGate;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Barrier,
    },
    thread,
    vec::Vec,
};

#[test]
fn closed_gate_rejects_callbacks() {
    let gate = CallbackGate::new();
    assert!(gate.try_enter().is_none());
    assert!(gate.is_idle());
    gate.open();
    gate.close();
    assert!(gate.try_enter().is_none());
    assert!(gate.is_idle());
}

#[test]
fn guard_serializes_and_releases_forwarding() {
    let gate = CallbackGate::new();
    gate.open();
    let guard = gate.try_enter().unwrap();
    assert!(gate.try_enter().is_none());
    assert!(!gate.is_idle());
    drop(guard);
    assert!(gate.is_idle());
    assert!(gate.try_enter().is_some());
}

#[test]
fn closing_an_active_callback_does_not_reopen_on_release() {
    let gate = CallbackGate::new();
    gate.open();
    let guard = gate.try_enter().unwrap();
    gate.close();
    assert!(!gate.is_idle());
    assert!(gate.try_enter().is_none());
    drop(guard);
    assert!(gate.is_idle());
    assert!(gate.try_enter().is_none());
}

#[test]
fn teardown_drains_callback_before_reclaiming_its_resource() {
    let gate = Arc::new(CallbackGate::new());
    let entered = Arc::new(Barrier::new(2));
    let finish = Arc::new(Barrier::new(2));
    let resource = Arc::new(AtomicUsize::new(0));
    gate.open();

    let worker = {
        let gate = Arc::clone(&gate);
        let entered = Arc::clone(&entered);
        let finish = Arc::clone(&finish);
        let resource = Arc::clone(&resource);
        thread::spawn(move || {
            let _guard = gate.try_enter().unwrap();
            entered.wait();
            finish.wait();
            resource.store(42, Ordering::Relaxed);
        })
    };

    entered.wait();
    gate.close();
    assert!(!gate.is_idle());
    assert!(gate.try_enter().is_none());
    finish.wait();
    while !gate.is_idle() {
        thread::yield_now();
    }
    assert_eq!(resource.load(Ordering::Relaxed), 42);
    assert!(gate.try_enter().is_none());
    worker.join().unwrap();
}

#[test]
fn simultaneous_callbacks_cannot_both_enter() {
    let gate = CallbackGate::new();
    let start = Barrier::new(3);
    let finish = Barrier::new(3);
    gate.open();
    thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    let guard = gate.try_enter();
                    finish.wait();
                    guard.is_some()
                })
            })
            .collect();
        start.wait();
        finish.wait();
        assert_eq!(
            workers
                .into_iter()
                .map(|w| w.join().unwrap())
                .filter(|entered| *entered)
                .count(),
            1
        );
    });
}

#[test]
fn racing_close_and_admission_cannot_use_a_reclaimed_resource() {
    for _ in 0..128 {
        let gate = CallbackGate::new();
        let start = Barrier::new(2);
        let alive = AtomicBool::new(true);
        gate.open();
        thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                if let Some(_guard) = gate.try_enter() {
                    assert!(alive.load(Ordering::Relaxed));
                }
            });
            start.wait();
            gate.close();
            while !gate.is_idle() {
                thread::yield_now();
            }
            alive.store(false, Ordering::Relaxed);
        });
        assert!(gate.try_enter().is_none());
    }
}
