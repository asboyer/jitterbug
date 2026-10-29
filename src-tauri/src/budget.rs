use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub reserved_usd: f64,
    pub reported_estimate_usd: f64,
    pub requests: u64,
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
        lock.try_lock_exclusive()
            .map_err(|_| "Another Jitterbug instance is updating spending; retry later.")?;
        let mut ledger: Ledger = crate::storage::read(&self.path)?;
        if !ledger.reserved_usd.is_finite()
            || ledger.reserved_usd < 0.0
            || !ledger.reported_estimate_usd.is_finite()
            || ledger.reported_estimate_usd < 0.0
        {
            return Err("Invalid spending ledger; cloud calls are blocked.".into());
        }
        update(&mut ledger)?;
        crate::storage::write(&self.path, &ledger)?;
        Ok(ledger)
    }
    pub fn reserve(&self, amount: f64, configured_limit: f64) -> Result<(), String> {
        let environment_limit =
            environment_ceiling(std::env::var("JITTERBUG_SPEND_LIMIT_USD").ok())?;
        let limit = configured_limit.min(environment_limit).min(100.0);
        if !limit.is_finite() || !amount.is_finite() || amount <= 0.0 || amount > 0.10 {
            return Err("Invalid budget or request exceeds the $0.10 per-call ceiling.".into());
        }
        self.mutate(|l| {
            if l.reserved_usd + amount > limit || l.requests >= 200 { return Err("Cloud spending or 200-request prototype limit reached. Local observations remain available.".into()); }
            l.reserved_usd += amount; l.requests += 1; Ok(())
        }).map(|_| ())
    }
    pub fn record_usage(&self, amount: f64) -> Result<(), String> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("Invalid provider usage.".into());
        }
        self.mutate(|l| {
            l.reported_estimate_usd += amount;
            l.reserved_usd = l.reserved_usd.max(l.reported_estimate_usd);
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
