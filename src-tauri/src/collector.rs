use crate::{attribution::Attribution, pressure::Detector, types::*};
use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub struct Collector {
    system: System,
    attribution: Attribution,
    detector: Detector,
    history: VecDeque<MetricPoint>,
    workload_history: HashMap<String, VecDeque<ResourcePoint>>,
    previous_swap: Option<u64>,
    samples: u64,
}

impl Default for Collector {
    fn default() -> Self {
        Self {
            system: System::new(),
            attribution: Attribution::default(),
            detector: Detector::default(),
            history: VecDeque::new(),
            workload_history: HashMap::new(),
            previous_swap: None,
            samples: 0,
        }
    }
}

impl Collector {
    pub fn sample(&mut self) -> Snapshot {
        let started = Instant::now();
        let metadata = self.samples.is_multiple_of(6);
        let refresh = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_user(UpdateKind::OnlyIfNotSet)
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cwd(if metadata {
                UpdateKind::Always
            } else {
                UpdateKind::OnlyIfNotSet
            });
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
        self.system.refresh_memory();
        self.system.refresh_cpu_usage();
        let cores = self.system.cpus().len().max(1);
        let mut groups: HashMap<String, Workload> = HashMap::new();
        let mut processes = Vec::new();
        for p in self.system.processes().values() {
            let label = self.attribution.label(p, &self.system);
            let cpu = (p.cpu_usage() / cores as f32).clamp(0.0, 100.0);
            let group = groups.entry(label.id.clone()).or_insert_with(|| Workload {
                id: label.id.clone(),
                label: label.label,
                category: label.category,
                description: label.description.clone(),
                attribution: label.confidence,
                ..Default::default()
            });
            group.cpu_percent += cpu;
            group.memory_bytes = group.memory_bytes.saturating_add(p.memory());
            group.process_count += 1;
            let actions = crate::actions::permitted(p, &self.system);
            let disk = p.disk_usage();
            processes.push(ProcessRecord {
                pid: p.pid().as_u32(),
                start_time: p.start_time(),
                parent_pid: p.parent().map(|id| id.as_u32()),
                name: p.name().to_string_lossy().chars().take(80).collect(),
                cpu_percent: cpu,
                memory_bytes: p.memory(),
                workload_id: label.id,
                description: label.description,
                evidence: label.evidence,
                actionable: !actions.is_empty(),
                actions,
                disk_read_bytes: if self.samples > 0 { disk.read_bytes } else { 0 },
                disk_written_bytes: if self.samples > 0 {
                    disk.written_bytes
                } else {
                    0
                },
            });
        }
        let total = self.system.total_memory().max(1);
        let score = |cpu: f32, memory: u64| cpu + (memory as f32 / total as f32) * 100.0;
        processes.sort_by(|a, b| {
            score(b.cpu_percent, b.memory_bytes).total_cmp(&score(a.cpu_percent, a.memory_bytes))
        });
        // Payloads and history remain bounded even on busy development machines.
        processes.truncate(4096);
        let mut workloads: Vec<_> = groups.into_values().collect();
        workloads.sort_by(|a, b| {
            score(b.cpu_percent, b.memory_bytes).total_cmp(&score(a.cpu_percent, a.memory_bytes))
        });
        workloads.truncate(512);
        let captured_at = now();
        self.workload_history
            .retain(|id, _| workloads.iter().any(|w| &w.id == id));
        for workload in &mut workloads {
            let history = self
                .workload_history
                .entry(workload.id.clone())
                .or_default();
            if self.samples > 0 {
                history.push_back(ResourcePoint {
                    time: captured_at,
                    cpu_percent: workload.cpu_percent,
                    memory_bytes: workload.memory_bytes,
                });
                if history.len() > 24 {
                    history.pop_front();
                }
            }
            workload.history = history.iter().cloned().collect();
        }
        let swap = self.system.used_swap();
        let mut snapshot = Snapshot {
            source: "live".into(),
            captured_at,
            sample_ready: self.samples > 0,
            interval_ms: 5000,
            cpu_percent: self.system.global_cpu_usage().clamp(0.0, 100.0),
            logical_cpus: cores,
            memory_total_bytes: total,
            memory_available_bytes: self.system.available_memory(),
            swap_used_bytes: swap,
            swap_growth_bytes: self
                .previous_swap
                .map(|old| swap.saturating_sub(old))
                .unwrap_or_default(),
            process_count: self.system.processes().len(),
            processes,
            workloads,
            platform: crate::platform::readings(),
            ..Default::default()
        };
        (snapshot.pressure, snapshot.incident) = self.detector.update(&snapshot);
        if snapshot.sample_ready {
            self.history.push_back(MetricPoint {
                time: snapshot.captured_at,
                cpu_percent: snapshot.cpu_percent,
                memory_percent: 100.0
                    * (1.0 - snapshot.memory_available_bytes as f32 / total as f32),
            });
        }
        if self.history.len() > 120 {
            self.history.pop_front();
        }
        snapshot.history = self.history.iter().cloned().collect();
        snapshot.collector_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.previous_swap = Some(swap);
        self.samples += 1;
        snapshot
    }
}

pub fn explain(snapshot: &Snapshot, workload_id: Option<&str>) -> String {
    let gib = |bytes: u64| bytes as f64 / 1073741824.0;
    let mut lines = vec![format!(
        "CPU is at {:.0}% of total capacity. {:.1} GiB of {:.1} GiB memory is available.",
        snapshot.cpu_percent,
        gib(snapshot.memory_available_bytes),
        gib(snapshot.memory_total_bytes)
    )];
    if snapshot.pressure.reasons.is_empty() {
        lines.push("The current sample does not show system-wide resource pressure. A single slow task can still have its own bottleneck.".into());
    } else {
        lines.extend(snapshot.pressure.reasons.iter().cloned());
    }
    for w in snapshot
        .workloads
        .iter()
        .filter(|w| workload_id.is_none_or(|id| w.id == id))
        .take(3)
    {
        lines.push(format!(
            "{}: {:.0}% CPU, {:.1} GiB summed process memory across {} processes. {} Evidence: {}.",
            w.label,
            w.cpu_percent,
            gib(w.memory_bytes),
            w.process_count,
            w.description,
            w.attribution
        ));
    }
    if snapshot.swap_used_bytes > 0 {
        lines.push("Existing swap use alone does not prove current thrashing; watch its growth and OS memory pressure.".into());
    }
    lines.push("These are deterministic observations, not a model diagnosis. Shared memory can be counted in multiple process totals. No processes have been changed.".into());
    lines.join("\n\n")
}
