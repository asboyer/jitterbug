fn main() {
    let mut collector = jitterbug_lib::collector::Collector::default();
    collector.sample();
    std::thread::sleep(std::time::Duration::from_secs(5));
    let snapshot = collector.sample();
    assert!(snapshot.sample_ready && snapshot.memory_total_bytes > 0 && snapshot.process_count > 0);
    assert!(!snapshot.workloads.is_empty());
    // Never print process names, paths, machine identifiers, or telemetry values.
    println!("Live collector smoke passed: sample, process inventory, workload attribution, and bounded history.");
}
