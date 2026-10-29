use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlatformReadings {
    pub os: String,
    pub memory_pressure: Option<String>,
    pub thermal: Option<String>,
    pub process_actions: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub pid: u32,
    pub start_time: u64,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub workload_id: String,
    pub description: String,
    pub evidence: Vec<String>,
    pub actionable: bool,
    pub actions: Vec<String>,
    pub disk_read_bytes: u64,
    pub disk_written_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourcePoint {
    pub time: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionReport {
    pub action: String,
    pub message: String,
    pub before: ProcessRecord,
    pub after: Option<ProcessRecord>,
    pub measured_after_seconds: u64,
    pub fresh: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Workload {
    pub id: String,
    pub label: String,
    pub category: String,
    pub description: String,
    pub attribution: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
    pub history: Vec<ResourcePoint>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pressure {
    pub level: String,
    pub reasons: Vec<String>,
    pub sustained_samples: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Incident {
    pub id: u64,
    pub created_at: u64,
    pub title: String,
    pub reasons: Vec<String>,
    pub workload_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MetricPoint {
    pub time: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub source: String,
    pub captured_at: u64,
    pub sample_ready: bool,
    pub interval_ms: u64,
    pub cpu_percent: f32,
    pub logical_cpus: usize,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_growth_bytes: u64,
    pub process_count: usize,
    pub processes: Vec<ProcessRecord>,
    pub workloads: Vec<Workload>,
    pub platform: PlatformReadings,
    pub pressure: Pressure,
    pub incident: Option<Incident>,
    pub history: Vec<MetricPoint>,
    pub collector_ms: f64,
}

impl Snapshot {
    /// Keep the frequently emitted dashboard small; retain the wider inventory for questions.
    pub fn dashboard_view(&self) -> Self {
        let mut view = self.clone();
        view.processes.truncate(160);
        view.workloads.truncate(40);
        view
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
