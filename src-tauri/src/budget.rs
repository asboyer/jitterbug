use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub reserved_usd: f64,
    pub reported_estimate_usd: f64,
    pub requests: u64,
    #[serde(default)]
    pending: BTreeMap<u64, f64>,
}

pub struct Budget {
    path: PathBuf,
}

impl Budget {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("spending.json"),
        }
    }
    pub fn status(&self) -> Result<Ledger, String> {
        crate::storage::read(&self.path)
    }
    fn mutate(
        &self,
        update: impl FnOnce(&mut Ledger) -> Result<(), String>,
    ) -> Result<Ledger, String> {
        std::fs::create_dir_all(self.path.parent().unwrap())
            .map_err(|_| "Cannot create budget directory.")?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(self.path.with_extension("lock"))
            .map_err(|_| "Cannot open spending lock.")?;
        // A child process spawned by another thread holds a copy of this descriptor until it
        // starts, so a just-released lock can look taken for a moment. Retry briefly; another
        // instance holding it for longer still fails closed.
        let mut attempts = 0;
        while lock.try_lock_exclusive().is_err() {
            attempts += 1;
            if attempts >= 20 {
                return Err("Another Jitterbug instance is updating spending; retry later.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let mut ledger: Ledger = crate::storage::read(&self.path)?;
        if !ledger.reserved_usd.is_finite()
            || ledger.reserved_usd < 0.0
            || !ledger.reported_estimate_usd.is_finite()
            || ledger.reported_estimate_usd < 0.0
            || ledger.pending.len() > 200
            || ledger
                .pending
                .values()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 0.10)
            || ledger.pending.values().sum::<f64>() > ledger.reserved_usd + 0.000001
        {
            return Err("Invalid spending ledger; cloud calls are blocked.".into());
        }
        update(&mut ledger)?;
        crate::storage::write(&self.path, &ledger)?;
        Ok(ledger)
    }
    pub fn reserve(&self, amount: f64, configured_limit: f64) -> Result<u64, String> {
        let environment_limit =
            environment_ceiling(std::env::var("JITTERBUG_SPEND_LIMIT_USD").ok())?;
        let limit = configured_limit.min(environment_limit).min(100.0);
        if !limit.is_finite() || !amount.is_finite() || amount <= 0.0 || amount > 0.10 {
            return Err("Invalid budget or request exceeds the $0.10 per-call ceiling.".into());
        }
        self.mutate(|l| {
            if l.requests >= 200 {
                return Err(
                    "The 200-request prototype limit was reached. AI investigation is unavailable."
                        .into(),
                );
            }
            if l.reserved_usd + amount > limit {
                let remedy = if environment_limit < configured_limit {
                    "The local environment spending ceiling is lower than Settings."
                } else {
                    "Review the cloud budget in Settings."
                };
                return Err(format!(
                    "The ${limit:.2} cloud budget cannot cover this request. {remedy}"
                ));
            }
            l.reserved_usd += amount;
            l.requests += 1;
            l.pending.insert(l.requests, amount);
            Ok(())
        })
        .map(|l| l.requests)
    }
    /// Settle exactly one successful request. Unknown usage and failed requests retain
    /// their reservation, including across restarts. Legacy reservations are untouched.
    pub fn settle(&self, request: u64, amount: f64) -> Result<(), String> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("Invalid provider usage.".into());
        }
        self.mutate(|l| {
            let reserved = l
                .pending
                .remove(&request)
                .ok_or("Spending reservation is unavailable or already settled.")?;
            l.reported_estimate_usd += amount;
            l.reserved_usd = (l.reserved_usd - reserved + amount).max(l.reported_estimate_usd);
            Ok(())
        })
        .map(|_| ())
    }
}

fn environment_ceiling(value: Option<String>) -> Result<f64, String> {
    match value {
        None => Ok(100.0),
        Some(value) => value
            .parse::<f64>()
            .ok()
            .filter(|limit| limit.is_finite() && *limit >= 0.0)
            .ok_or_else(|| "Invalid environment spending ceiling; cloud calls are blocked.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_requests_release_only_their_own_unused_reservation() {
        let path = std::env::temp_dir().join(format!(
            "jitterbug-budget-settlement-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let b = Budget::new(&path);
        let first = b.reserve(0.08, 0.10).unwrap();
        assert!(b.reserve(0.03, 0.10).is_err());
        b.settle(first, 0.004).unwrap();
        assert!((b.status().unwrap().reserved_usd - 0.004).abs() < 1e-9);
        assert!(b.settle(first, 0.004).is_err()); // no double refund
        let failed = b.reserve(0.08, 0.10).unwrap();
        let b = Budget::new(&path);
        assert!(b.status().unwrap().pending.contains_key(&failed));
        assert!(b.reserve(0.03, 0.10).is_err()); // timeout/unknown charge remains reserved
        assert_eq!(b.status().unwrap().requests, 2);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn legacy_unknown_charges_are_not_refunded_during_migration() {
        let path =
            std::env::temp_dir().join(format!("jitterbug-budget-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("spending.json"),
            r#"{"reserved_usd":1.9,"reported_estimate_usd":0.2,"requests":50}"#,
        )
        .unwrap();
        let b = Budget::new(&path);
        let id = b.reserve(0.08, 2.0).unwrap();
        b.settle(id, 0.004).unwrap();
        assert!((b.status().unwrap().reserved_usd - 1.904).abs() < 1e-9);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn invalid_environment_ceiling_cannot_disable_the_limit() {
        for value in ["NaN", "inf", "-1", "invalid", ""] {
            assert!(environment_ceiling(Some(value.into())).is_err());
        }
        assert_eq!(environment_ceiling(Some("0".into())).unwrap(), 0.0);
        assert_eq!(environment_ceiling(None).unwrap(), 100.0);
    }
    #[test]
    fn reservations_survive_restarts_and_corruption_fails_closed() {
        let path =
            std::env::temp_dir().join(format!("jitterbug-budget-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let b = Budget::new(&path);
        b.reserve(0.01, 0.02).unwrap();
        let b = Budget::new(&path);
        b.reserve(0.01, 0.02).unwrap();
        assert!(b.reserve(0.01, 0.02).is_err());
        std::fs::write(path.join("spending.json"), "broken").unwrap();
        assert!(b.reserve(0.01, 1.0).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
