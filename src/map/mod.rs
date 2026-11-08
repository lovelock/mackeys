//! TOML config + token DSL + built-in tables. Task 4 (core plan).

pub mod builtins;
pub mod tokens;
pub mod config;

pub use builtins::{build_builtins, LauncherSpec, BUILTIN_IDS};
pub use config::{load_config, Config};

#[cfg(test)]
mod tests;

/// Compile one context's rule set: user rules first, then builtins (minus disabled).
pub fn compile(kb: crate::types::Kb, ctx: crate::types::Ctx, cfg: &Config) -> crate::types::RuleSet {
    let launcher = builtins::LauncherSpec::from_cfg(&cfg.launcher);
    let mut rules: Vec<crate::types::Rule> = Vec::new();
    for ur in &cfg.rules {
        if !ctx_matches(&ur.context, ctx) {
            continue;
        }
        if let (Ok(input), Ok(output)) = (
            crate::map::config::parse_combo_str(&ur.input),
            crate::map::config::parse_output(&ur.output),
        ) {
            rules.push(crate::types::Rule {
                id: format!("user:{}", ur.input),
                builtin: false,
                sticky: false,
                context: ctx,
                input,
                output,
            });
        }
    }
    for mut r in builtins::build_builtins(kb, ctx, &launcher) {
        if cfg.general.disable_builtins.iter().any(|d| *d == r.id) {
            continue;
        }
        rules.push(r);
    }
    crate::types::RuleSet { rules }
}

fn ctx_matches(spec: &str, ctx: crate::types::Ctx) -> bool {
    match spec {
        "any" => true,
        "gui" => ctx == crate::types::Ctx::Gui,
        "term" => ctx == crate::types::Ctx::Terminal,
        _ => true, // validation rejects others
    }
}

pub fn compile_all(kb: crate::types::Kb, cfg: &Config)
    -> (crate::types::RuleSet, crate::types::RuleSet)
{
    (compile(kb, crate::types::Ctx::Gui, cfg), compile(kb, crate::types::Ctx::Terminal, cfg))
}

pub fn dump() -> anyhow::Result<()> {
    println!("mackeys dump — compiled rule sets (first-match-wins order)");
    let cfg = crate::map::config::load_config(
        &crate::setup::config_path()?,
    )?;
    let kb = match cfg.general.kbtype_default.as_str() { "apple" => crate::types::Kb::Apple, _ => crate::types::Kb::Windows };
    let (gui, term) = compile_all(kb, &cfg);
    println!("kbtype: {kb:?}");
    println!("── GUI context ──");
    for (i, r) in gui.rules.iter().enumerate() { println!("{i:>3}  {:<28} {:?} sticky={} builtin={}", r.id, r.input, r.sticky, r.builtin); }
    println!("── Terminal context ──");
    for (i, r) in term.rules.iter().enumerate() { println!("{i:>3}  {:<28} {:?} sticky={} builtin={}", r.id, r.input, r.sticky, r.builtin); }
    Ok(())
}
