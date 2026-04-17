use system_monitor_web::derive_gpu_activity_metric;

#[test]
fn uses_rc6_residency_as_primary_signal() {
    let result = derive_gpu_activity_metric(1000.0, Some(250.0), 400.0, 1300.0, 400.0);
    assert_eq!(result.utilization, 75);
    assert_eq!(result.source, "rc6-residency");
}

#[test]
fn clamps_impossible_rc6_deltas() {
    let result = derive_gpu_activity_metric(1000.0, Some(5000.0), 400.0, 1300.0, 400.0);
    assert_eq!(result.utilization, 0);
    assert_eq!(result.source, "rc6-residency");
}

#[test]
fn falls_back_to_frequency_when_rc6_unavailable() {
    let result = derive_gpu_activity_metric(0.0, None, 400.0, 1300.0, 850.0);
    assert_eq!(result.utilization, 50);
    assert_eq!(result.source, "frequency-fallback");
}
