use spectra_flux::wasm::{
    CircuitBreakerConfig, CircuitPermission, FluxcellWasmConfig, WasmCircuitBreaker, WasmHost,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn test_instance_pooling_concurrency_saturation() {
    let wat_echo = r#"
    (module
      (memory (export "memory") 1)
      (data (i32.const 2048) "{\"status\":200,\"body\":\"pooled_ok\"}")
      (func (export "allocate") (param i32) (result i32) i32.const 1024)
      (func (export "deallocate") (param i32 i32))
      (func (export "handle_http") (param i32 i32) (result i64)
        ;; High 32 bits = 2048, Low 32 bits = 33 -> (2048 << 32) | 33 = 8796093022241
        i64.const 8796093022241
      )
    )
    "#;

    let host = Arc::new(WasmHost::new(5, None).expect("Failed to initialize WasmHost"));
    let cfg = FluxcellWasmConfig {
        max_instances: 4, // 4 instances in the pool
        timeout_ms: 2000,
        ..Default::default()
    };
    host.register_wat("pool-saturation-cell", wat_echo, cfg)
        .expect("Registration failed");

    // Spawn 16 concurrent threads (4x pool capacity)
    let num_threads = 16;
    let mut handles = Vec::new();
    let success_count = Arc::new(AtomicU32::new(0));

    for _ in 0..num_threads {
        let h = host.clone();
        let counter = success_count.clone();
        handles.push(std::thread::spawn(move || {
            let (status, _, body) = h
                .invoke_http("pool-saturation-cell", "/echo", "GET", vec![], vec![])
                .expect("Invoke failed");
            assert_eq!(status, 200);
            assert_eq!(body, b"pooled_ok");
            counter.fetch_add(1, Ordering::SeqCst);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(
        success_count.load(Ordering::SeqCst),
        num_threads,
        "All 16 concurrent requests must succeed via pooling"
    );
}

#[test]
fn test_epoch_interruption_on_infinite_loop() {
    let wat_infinite = r#"
    (module
      (memory (export "memory") 1)
      (func (export "allocate") (param i32) (result i32) i32.const 0)
      (func (export "deallocate") (param i32 i32))
      (func (export "handle_http") (param i32 i32) (result i64)
        (loop (br 0))
        i64.const 0
      )
    )
    "#;

    let host = Arc::new(WasmHost::new(5, None).expect("Failed to initialize WasmHost"));
    let cfg = FluxcellWasmConfig {
        timeout_ms: 100, // Preemptive timeout after 100ms
        ..Default::default()
    };
    host.register_wat("infinite-loop-cell", wat_infinite, cfg)
        .expect("Registration failed");

    let start = Instant::now();
    let result = host.invoke_http("infinite-loop-cell", "/loop", "GET", vec![], vec![]);
    let elapsed = start.elapsed();

    // Must return an error due to epoch interruption
    assert!(result.is_err(), "Infinite loop must be preemptively terminated");
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("timeout") || err_str.contains("interrupt") || err_str.contains("trap"),
        "Error message should mention timeout or interruption, got: {}",
        err_str
    );

    // Must finish quickly, well under 2 seconds
    assert!(
        elapsed < Duration::from_secs(2),
        "Epoch watchdog took too long to terminate loop: {:?}",
        elapsed
    );
}

#[test]
fn test_circuit_breaker_full_state_machine_lifecycle() {
    let cb = Arc::new(WasmCircuitBreaker::new(CircuitBreakerConfig {
        consecutive_failure_threshold: 3,
        cooloff_duration: Duration::from_millis(50),
    }));

    // Initial state: Closed
    assert_eq!(cb.can_execute(), CircuitPermission::Allow);
    assert!(!cb.is_open());

    // Record 2 failures -> Still Closed
    cb.record_failure();
    cb.record_failure();
    assert_eq!(cb.can_execute(), CircuitPermission::Allow);
    assert!(!cb.is_open());

    // 3rd failure trips the breaker -> Open
    cb.record_failure();
    assert!(cb.is_open());
    assert_eq!(cb.can_execute(), CircuitPermission::Denied);

    // Wait for cooloff duration
    std::thread::sleep(Duration::from_millis(60));

    // After cooloff -> HalfOpen allows a single Canary Probe
    assert_eq!(cb.can_execute(), CircuitPermission::Probe);
    // Subsequent calls while probe is in flight must be Denied
    assert_eq!(cb.can_execute(), CircuitPermission::Denied);

    // Probe succeeds -> Returns to Closed
    cb.record_success();
    assert!(!cb.is_open());
    assert_eq!(cb.can_execute(), CircuitPermission::Allow);
}

#[test]
fn test_circuit_breaker_half_open_failure_reopens_immediately() {
    let cb = Arc::new(WasmCircuitBreaker::new(CircuitBreakerConfig {
        consecutive_failure_threshold: 3,
        cooloff_duration: Duration::from_millis(50),
    }));

    // Trip the breaker
    cb.record_failure();
    cb.record_failure();
    cb.record_failure();
    assert!(cb.is_open());

    // Wait for cooloff
    std::thread::sleep(Duration::from_millis(60));

    // Canary probe allowed
    assert_eq!(cb.can_execute(), CircuitPermission::Probe);

    // Canary probe fails -> Re-opens immediately without waiting for threshold
    cb.record_failure();
    assert!(cb.is_open());
    assert_eq!(cb.can_execute(), CircuitPermission::Denied);
}
