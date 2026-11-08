//! Built-in keymap tables — GUI family (spec §3.1) and terminal family (§3.2).

use crate::types::{Combo, Ctx, Kb, OutputSel, Rule, RuleSet};

pub const BUILTIN_IDS: &[&str] = &[
    "cmd_tab",
    "cmd_shift_tab",
    "cmd_grave",
    "cmd_space",
    "cmd_h",
    "cmd_q",
    "cmd_arrow_left",
    "cmd_arrow_right",
    "cmd_arrow_up",
    "cmd_arrow_down",
    "wordwise_opt_backspace",
    "wordwise_opt_delete",
    "wordwise_opt_left",
    "wordwise_opt_right",
    "term_cmd_c",
    "term_cmd_v",
    "term_cmd_letters",
    "term_cmd_nums",
    "term_cmd_arrows",
    // VSCode hybrid family (app-scoped: editor + embedded terminal in one window)
    "vsc_grave",
    "vsc_shift_grave",
    "vsc_cmd_dot",
];

#[derive(Debug, Clone, PartialEq)]
pub enum LauncherSpec {
    /// output combo, e.g. ["alt","space"] for KRunner
    Combo(Combo),
    /// swallow (does NOT fall through — fcitx5 safety, spec §8)
    Swallow,
}

impl LauncherSpec {
    pub fn from_cfg(l: &crate::map::config::Launcher) -> Self {
        crate::map::config::launcher_spec(&l.cmd_space).unwrap_or(LauncherSpec::Swallow)
    }
}

fn m(v: &[u16]) -> Vec<u16> {
    v.to_vec()
}

fn rule(
    id: &'static str,
    context: Ctx,
    input_mods: Vec<u16>,
    input_key: u16,
    output: OutputSel,
) -> Rule {
    Rule {
        id: id.to_string(),
        builtin: true,
        sticky: false,
        context,
        input: Combo {
            mods: input_mods,
            key: Some(input_key),
        },
        output,
        apps: None,
    }
}

fn sticky_rule(
    id: &'static str,
    context: Ctx,
    input_mods: Vec<u16>,
    input_key: u16,
    output: OutputSel,
) -> Rule {
    let mut r = rule(id, context, input_mods, input_key, output);
    r.sticky = true;
    r
}

/// The full table for one context + launcher spec. `cmd_space`'s "none" is
/// a swallow (never a fall-through — fcitx5 treats Ctrl-ish combos, spec §8).
pub fn build_builtins(kb: Kb, context: Ctx, launcher: &LauncherSpec) -> Vec<Rule> {
    let _ = kb; // aliases are kb-independent (output-side codes)
    let cmd = vec![crate::engine::R_CTRL];
    let cmdshift = vec![crate::engine::R_CTRL, 42];
    let opt = vec![crate::engine::L_ALT];
    let optshift = vec![crate::engine::L_ALT, 42];
    let rs = RuleSet::default();
    let _ = rs;

    let mut v: Vec<Rule> = Vec::new();

    // ---- wheel-level: same rule ids in both contexts ----
    match launcher {
        LauncherSpec::Combo(out) => {
            v.push(sticky_rule("cmd_space", context, m(&cmd), 57, OutputSel::Combo(out.clone())));
        }
        LauncherSpec::Swallow => {
            v.push(rule("cmd_space", context, m(&cmd), 57, OutputSel::Ignore));
        }
    }
    v.push(sticky_rule(
        "cmd_tab",
        context,
        m(&cmd),
        15,
        OutputSel::Combo(Combo { mods: m(&[56]), key: Some(15) }),
    ));
    v.push(sticky_rule(
        "cmd_shift_tab",
        context,
        m(&cmdshift),
        15,
        OutputSel::Combo(Combo { mods: m(&[56, 42]), key: Some(15) }),
    ));
    v.push(sticky_rule(
        "cmd_grave",
        context,
        m(&cmd),
        41,
        OutputSel::Combo(Combo { mods: m(&[56]), key: Some(41) }),
    ));

    match context {
        Ctx::Gui => {
            v.push(rule("cmd_h", context, m(&cmd), 35, OutputSel::Ignore));
            v.push(rule(
                "cmd_q",
                context,
                m(&cmd),
                16,
                OutputSel::Combo(Combo { mods: m(&[56]), key: Some(62) }), // Alt+F4
            ));
            v.push(rule("cmd_arrow_left", context, m(&cmd), 105, OutputSel::Combo(Combo { mods: vec![], key: Some(102) })));
            v.push(rule("cmd_arrow_right", context, m(&cmd), 106, OutputSel::Combo(Combo { mods: vec![], key: Some(107) })));
            v.push(rule("cmd_arrow_up", context, m(&cmd), 103, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(102) })));
            v.push(rule("cmd_arrow_down", context, m(&cmd), 108, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(107) })));
            v.push(rule("wordwise_opt_backspace", context, m(&opt), 14, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(14) })));
            v.push(rule("wordwise_opt_delete", context, m(&opt), 119, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(119) })));
            v.push(rule("wordwise_opt_left", context, m(&opt), 105, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(105) })));
            v.push(rule("wordwise_opt_right", context, m(&opt), 106, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(106) })));
            v.push(rule("wordwise_opt_shift_backspace", context, m(&optshift), 14, OutputSel::Combo(Combo { mods: m(&[29, 42]), key: Some(14) })));
            v.push(rule("cmd_comma", context, m(&cmd), 51, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(51) })));
        }
        Ctx::Terminal => {
            // Cmd+letters → Ctrl+Shift+letters (C/V explicit for readability)
            v.push(rule("term_cmd_c", context, m(&cmd), 46, OutputSel::Combo(Combo { mods: m(&[29, 42]), key: Some(46) })));
            v.push(rule("term_cmd_v", context, m(&cmd), 47, OutputSel::Combo(Combo { mods: m(&[29, 42]), key: Some(47) })));
            for c in b'a'..=b'z' {
                if c == b'c' || c == b'v' {
                    continue;
                }
                let phys = crate::map::tokens::resolve_token(
                    std::str::from_utf8(&[c]).unwrap(),
                    Kb::Windows,
                )
                .expect("letter code");
                v.push(rule(
                    "term_cmd_letters",
                    context,
                    m(&cmd),
                    phys,
                    OutputSel::Combo(Combo { mods: m(&[29, 42]), key: Some(phys) }),
                ));
            }
            // digits: Cmd+0..9 → Ctrl+0..9
            let digit_codes: [u16; 10] = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            for code in digit_codes {
                v.push(rule(
                    "term_cmd_nums",
                    context,
                    m(&cmd),
                    code,
                    OutputSel::Combo(Combo { mods: m(&[29]), key: Some(code) }),
                ));
            }
            // arrows → Ctrl+PageUp/PageDown (Toshy "General Terminals" nav)
            v.push(rule("term_cmd_arrows", context, m(&cmd), 105, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(104) })));
            v.push(rule("term_cmd_arrows", context, m(&cmd), 106, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(109) })));
            // wordwise survives in terminals too (overlay semantics, spec §8)
            v.push(rule("wordwise_opt_backspace", context, m(&opt), 14, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(14) })));
            v.push(rule("wordwise_opt_delete", context, m(&opt), 119, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(119) })));
            v.push(rule("wordwise_opt_left", context, m(&opt), 105, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(105) })));
            v.push(rule("wordwise_opt_right", context, m(&opt), 106, OutputSel::Combo(Combo { mods: m(&[29]), key: Some(106) })));
        }
    }
    // ---- VSCode hybrid family: app-scoped (Toshy "VSCodes" keymap port) ----
    let vsc_apps = vec!["code".to_string(), "code-oss".to_string(), "vscodium".to_string()];
    let vsc = |context: Ctx| -> Vec<Rule> {
        // Cmd+` → Ctrl+` : toggle the embedded terminal
        let mut v2 = vec![
            rule("vsc_grave", context, m(&cmd), 41,
                 OutputSel::Combo(Combo { mods: m(&[29]), key: Some(41) })),
            rule("vsc_shift_grave", context, m(&cmdshift), 41,
                 OutputSel::Combo(Combo { mods: m(&[29, 42]), key: Some(41) })),
            // Cmd+. → Ctrl+. : QuickFix (the injected keybindings.json turns
            // Ctrl+. with terminalFocus into the SIGINT sequence instead)
            rule("vsc_cmd_dot", context, m(&cmd), 52,
                 OutputSel::Combo(Combo { mods: m(&[29]), key: Some(52) })),
        ];
        for r in &mut v2 {
            r.apps = Some(vsc_apps.clone());
        }
        v2
    };
    v.extend(vsc(Ctx::Gui));
    v.extend(vsc(Ctx::Terminal));
    v
}
