//! Config file schema + loader (`~/.config/mackeys/config.toml`).
//! Defaults ARE the owner's preferences (spec §5); TOML is differential.

use crate::map::builtins::LauncherSpec;
use crate::types::{Combo, Kb, OutputSel};
use anyhow::Context as _;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub general: General,
    pub targets: Targets,
    pub launcher: Launcher,
    pub per_device: std::collections::BTreeMap<String, String>,
    pub rules: Vec<UserRule>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub kbtype_default: String,
    pub swap_caps_lctrl: bool,
    pub adapter: String,
    pub disable_builtins: Vec<String>,
    pub exclude_devices: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            kbtype_default: "windows".into(),
            swap_caps_lctrl: true,
            adapter: "auto".into(),
            disable_builtins: vec![],
            exclude_devices: vec![],
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Targets {
    pub terminal_classes: Vec<String>,
}

impl Default for Targets {
    /// Spec §3.2 default: omitting the key means the spec default list, not none.
    fn default() -> Self {
        Self {
            terminal_classes: [
                "^konsole", "ghostty", "alacritty", "kitty", "wezterm",
                "org.wezfurlong.wezterm", "foot", "rio",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Launcher {
    /// "none" swallows Cmd+Space; a token string like "alt+space" outputs it.
    pub cmd_space: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UserRule {
    pub input: String,
    pub output: String,
    #[serde(default = "default_any")]
    pub context: String,
    /// App-scope tokens (resourceClass/name contains, case-insensitive)
    #[serde(default)]
    pub apps: Option<Vec<String>>,
}

fn default_any() -> String {
    "any".into()
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    general: General,
    targets: Targets,
    launcher: Launcher,
    per_device: std::collections::BTreeMap<String, String>,
    rules: Vec<UserRule>,
}

impl Config {
    pub fn parse_toml(txt: &str) -> anyhow::Result<Self> {
        let raw: RawConfig = toml::from_str(txt)?;
        let parsed = Config {
            general: raw.general,
            targets: raw.targets,
            launcher: raw.launcher,
            per_device: raw.per_device,
            rules: raw.rules,
        };
        parsed.validate()?;
        Ok(parsed)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        use crate::map::tokens::resolve_token;
        match self.general.kbtype_default.as_str() {
            "windows" | "apple" => {}
            other => anyhow::bail!(
                "kbtype_default '{}' not supported in v1 (windows|apple)",
                other
            ),
        }
        if !["auto", "kwin", "gnome", "none", "windows"].contains(&self.general.adapter.as_str()) {
            anyhow::bail!("general.adapter '{}' invalid (auto|kwin|gnome|none|windows)", self.general.adapter);
        }
        // check launcher spec parses
        launcher_spec(&self.launcher.cmd_space)?;
        for r in &self.rules {
            parse_combo_str(&r.input)?;
            parse_output(&r.output)?;
            if !["gui", "term", "any"].contains(&r.context.as_str()) {
                anyhow::bail!("rules[{}].context '{}' invalid (gui|term|any)", r.input, r.context);
            }
        }
        for id in &self.general.disable_builtins {
            if !crate::map::builtins::BUILTIN_IDS.contains(&id.as_str()) {
                anyhow::bail!("disable_builtins: unknown builtin id '{}'", id);
            }
        }
        Ok(())
    }
}

/// Launcher[]cmd_space → LauncherSpec
pub fn launcher_spec(cmd_space: &str) -> anyhow::Result<LauncherSpec> {
    if cmd_space == "none" {
        return Ok(LauncherSpec::Swallow);
    }
    if cmd_space.is_empty() {
        return Ok(LauncherSpec::Combo(default_launcher()));
    }
    parse_combo_str(cmd_space).map(LauncherSpec::Combo)
}

pub fn default_launcher() -> Combo {
    // KRunner on this owner's machine resolves to Alt+Space (doctor verifies)
    Combo { mods: vec![56], key: Some(57) }
}

/// "cmd+c" / "ctrl+shift+z" → Combo (alias tokens)
pub fn parse_combo_str(s: &str) -> anyhow::Result<Combo> {
    use crate::map::tokens::resolve_token;
    let kb = Kb::Windows;
    let mut mods = Vec::new();
    let mut key = None;
    for (i, tok) in s.split('+').map(str::trim).enumerate() {
        let code = resolve_token(tok, kb)
            .with_context(|| format!("unknown token '{tok}' in combo '{s}'"))?;
        if i == s.split('+').count() - 1 && !["shift", "alt", "meta", "opt", "cmd", "ctrl", "caps", "lalt", "ralt", "lmeta", "lctrl"].contains(&tok) {
            key = Some(code);
        } else {
            // chained modifiers after the key? keep as mods too
            if key.is_none() {
                mods.push(code);
            } else {
                mods.push(code);
            }
        }
    }
    Ok(Combo { mods, key })
}

/// "ignore" | "none" → Ignore; else a combo → OutputSel::Combo
pub fn parse_output(s: &str) -> anyhow::Result<OutputSel> {
    if s == "ignore" || s == "none" {
        return Ok(OutputSel::Ignore);
    }
    parse_combo_str(s).map(OutputSel::Combo)
}

pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    let txt = std::fs::read_to_string(path)
        .with_context(|| format!("read config {}", path.display()))?;
    Config::parse_toml(&txt)
}
