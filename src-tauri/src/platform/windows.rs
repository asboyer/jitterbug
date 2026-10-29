use crate::types::PlatformReadings;

/// Process and memory collection already comes from the shared sysinfo collector.
/// Native pressure/thermal signals and validated process actions are the next Windows increment.
pub fn readings() -> PlatformReadings {
    PlatformReadings {
        os: "windows".into(),
        memory_pressure: None,
        thermal: None,
        process_actions: false,
    }
}
