//! `tamper.toml` — what to build, and what may change a ruling.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Directory holding the case files, relative to the repository root.
    pub cases: String,
    pub target: BTreeMap<String, Target>,
    pub cache: CacheCfg,
    pub lotse: LotseCfg,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// Flake attribute to build, e.g. `.#nixosConfigurations.server…toplevel`.
    pub attr: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCfg {
    /// Files that define the checks. A change here can flip ANY ruling, so
    /// they go into every cache key. Globs ending in `/**` are allowed.
    pub definitions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LotseCfg {
    /// Class registered once for the whole run (slots = 1).
    pub run_class: String,
    /// Class each individual build is queued under.
    pub build_class: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}
