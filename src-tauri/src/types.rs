use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlatformReadings {
    pub os: String,
    pub memory_pressure: Option<String>,
    pub thermal: Option<String>,
    pub process_actions: bool,
    pub memory: Option<MemoryBreakdown>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryBreakdown {
    pub app_bytes: u64,
    pub wired_bytes: u64,
    pub compressed_bytes: u64,
    pub cached_bytes: u64,
    pub used_bytes: u64,
    pub swap_in_total_bytes: u64,
    pub swap_out_total_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessRecord {
    // Private collection metadata. Only the explicit local detail view may expose paths;
    // telemetry events, stored incidents, action reports, and model prompts omit them.
    #[serde(skip)]
    pub executable: Option<std::path::PathBuf>,
    #[serde(skip)]
    pub working_directory: Option<std::path::PathBuf>,
    #[serde(skip)]
    pub parent_start_time: Option<u64>,
    #[serde(skip)]
    pub resource_role: String,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub cpu_cores: Vec<f32>,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_growth_bytes: u64,
    pub swap_in_bytes: Option<u64>,
    pub swap_out_bytes: Option<u64>,
    pub process_count: usize,
    pub processes: Vec<ProcessRecord>,
    pub workloads: Vec<Workload>,
    #[serde(default)]
    pub agent_workflows: Vec<crate::workflows::AgentWorkflow>,
    pub platform: PlatformReadings,
    pub pressure: Pressure,
    pub incident: Option<Incident>,
    pub history: Vec<MetricPoint>,
    pub collector_ms: f64,
}

impl Snapshot {
    /// Keep the frequently emitted dashboard small; retain the wider inventory for questions.
    pub fn dashboard_view(&self) -> Self {
        self.view(160, 40)
    }

    pub fn quick_view(&self) -> Self {
        self.view(0, 5)
    }

    fn view(&self, process_limit: usize, workload_limit: usize) -> Self {
        Self {
            source: self.source.clone(),
            captured_at: self.captured_at,
            sample_ready: self.sample_ready,
            interval_ms: self.interval_ms,
            cpu_percent: self.cpu_percent,
            logical_cpus: self.logical_cpus,
            cpu_cores: self.cpu_cores.clone(),
            memory_total_bytes: self.memory_total_bytes,
            memory_available_bytes: self.memory_available_bytes,
            swap_used_bytes: self.swap_used_bytes,
            swap_growth_bytes: self.swap_growth_bytes,
            swap_in_bytes: self.swap_in_bytes,
            swap_out_bytes: self.swap_out_bytes,
            process_count: self.process_count,
            processes: self.processes.iter().take(process_limit).cloned().collect(),
            workloads: self
                .workloads
                .iter()
                .take(workload_limit)
                .cloned()
                .collect(),
            agent_workflows: if process_limit > 0 {
                self.agent_workflows.clone()
            } else {
                Vec::new()
            },
            platform: self.platform.clone(),
            pressure: self.pressure.clone(),
            incident: self.incident.clone(),
            history: self.history.clone(),
            collector_ms: self.collector_ms,
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
