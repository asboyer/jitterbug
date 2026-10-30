use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Ollama,
    Meta,
    Anthropic,
    Jev,
}

impl Provider {
    pub fn local(&self) -> bool {
        *self == Self::Ollama
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::Meta => "meta",
            Self::Anthropic => "anthropic",
            Self::Jev => "jev",
        }
    }
    pub fn env(&self) -> &'static str {
        match self {
            Self::Ollama => "",
            Self::Meta => "META_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::Jev => "JEV_API_KEY",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Hybrid,
    LocalOnly,
    CloudOnly,
}

impl Mode {
    pub fn permits(&self, provider: &Provider) -> bool {
        match self {
            Self::Hybrid => true,
            Self::LocalOnly => provider.local(),
            Self::CloudOnly => !provider.local(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    pub provider: Provider,
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub decision: Tier,
    pub cheap: Tier,
    pub intelligent: Tier,
    pub decision_enabled: bool,
    pub budget_usd: f64,
    pub automation: crate::automation::Policy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Hybrid,
            decision: Tier {
                provider: Provider::Ollama,
                model: "tev1:0.8b".into(),
            },
            cheap: Tier {
                provider: Provider::Ollama,
                model: "qwen3.5:0.8b".into(),
            },
            intelligent: Tier {
                provider: Provider::Meta,
                model: "muse-spark-1.3".into(),
            },
            decision_enabled: false,
            budget_usd: 2.0,
            automation: crate::automation::Policy::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        self.automation.validate()?;
        if !self.budget_usd.is_finite() || !(0.0..=100.0).contains(&self.budget_usd) {
            return Err("Budget must be between $0 and $100.".into());
        }
        for (name, tier) in [
            ("decision", &self.decision),
            ("cheap", &self.cheap),
            ("intelligent", &self.intelligent),
        ] {
            if tier.model.is_empty()
                || tier.model.len() > 120
                || tier.model.chars().any(char::is_control)
            {
                return Err("Choose a valid model identifier.".into());
            }
            if tier.provider == Provider::Jev && name != "decision" {
                return Err("Jev supports the decision tier only.".into());
            }
            if !tier.provider.local() {
                price(tier)?;
            }
        }
        Ok(())
    }
}

// Explicit, reviewable prices. Unknown cloud models fail closed instead of guessing a charge.
// USD / 1M tokens; checked against provider documentation on 2026-10-28.
pub fn price(tier: &Tier) -> Result<(f64, f64), String> {
    match (&tier.provider, tier.model.as_str()) {
        (Provider::Ollama, _) => Ok((0.0, 0.0)),
        (Provider::Meta, "muse-spark-1.3" | "muse-spark-1.2" | "muse-spark-1.1") => Ok((1.25, 4.25)),
        (Provider::Anthropic, "claude-haiku-5-5") => Ok((0.10, 0.50)),
        (Provider::Anthropic, "claude-sonnet-5-5") => Ok((2.0, 10.0)),
        (Provider::Jev, "jev-1.14.0") => Ok((0.042, 0.0)),
        _ => Err("This cloud model has no reviewed price. Add its ID and current pricing in settings.rs before enabling it.".into()),
    }
}

pub fn key(provider: &Provider) -> Result<String, String> {
    if let Ok(value) = std::env::var(provider.env()) {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    keyring::Entry::new("com.jitterbug.desktop", provider.name())
        .and_then(|entry| entry.get_password())
        .map_err(|_| format!("No {} credential is configured.", provider.name()))
}

pub fn save_key(provider: Provider, value: &str) -> Result<(), String> {
    if provider.local()
        || value.len() < 8
        || value.len() > 4096
        || value.chars().any(char::is_whitespace)
    {
        return Err("Invalid provider credential.".into());
    }
    keyring::Entry::new("com.jitterbug.desktop", provider.name())
        .and_then(|entry| entry.set_password(value))
        .map_err(|_| "The operating-system credential store rejected this key.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_are_hard_boundaries() {
        for p in [Provider::Meta, Provider::Anthropic, Provider::Jev] {
            assert!(!Mode::LocalOnly.permits(&p));
            assert!(Mode::CloudOnly.permits(&p));
        }
        assert!(!Mode::CloudOnly.permits(&Provider::Ollama));
        assert!(Mode::LocalOnly.permits(&Provider::Ollama));
    }
    #[test]
    fn unknown_prices_and_invalid_budgets_are_rejected() {
        let mut s = Settings {
            budget_usd: f64::NAN,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.budget_usd = 101.0;
        assert!(s.validate().is_err());
        s.budget_usd = 2.0;
        s.intelligent.model = "unpriced-model".into();
        assert!(s.validate().is_err());
    }
}
