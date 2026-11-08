//! Token DSL + builtins + compiler tests (Task 4, core plan).

use crate::map::tokens::resolve_token;
use crate::types::{Ctx, Kb};

#[test]
fn resolve_aliases_to_output_codes() {
    // T2 ruling: rule-side aliases resolve to POST-modmap output codes, the
    // same values for both kbtypes (only the physical origin differs).
    for kb in [Kb::Windows, Kb::Apple] {
        assert_eq!(resolve_token("cmd", kb), Some(97), "cmd = RIGHT_CTRL");
        assert_eq!(resolve_token("ctrl", kb), Some(29), "ctrl = LEFT_CTRL");
        assert_eq!(resolve_token("caps", kb), Some(29), "caps alias = L_CTRL");
        assert_eq!(resolve_token("opt", kb), Some(56), "opt = LEFT_ALT");
        assert_eq!(resolve_token("alt", kb), Some(56));
        assert_eq!(resolve_token("meta", kb), Some(56));
        assert_eq!(resolve_token("shift", kb), Some(42));
        // letters and literals
        assert_eq!(resolve_token("c", kb), Some(46));
        assert_eq!(resolve_token("space", kb), Some(57));
        assert_eq!(resolve_token("tab", kb), Some(15));
        assert_eq!(resolve_token("grave", kb), Some(41));
        assert_eq!(resolve_token("h", kb), Some(35));
        assert_eq!(resolve_token("q", kb), Some(16));
        assert_eq!(resolve_token("left", kb), Some(105));
        assert_eq!(resolve_token("right", kb), Some(106));
        assert_eq!(resolve_token("home", kb), Some(102));
        assert_eq!(resolve_token("end", kb), Some(107));
        assert_eq!(resolve_token("pageup", kb), Some(104));
        assert_eq!(resolve_token("pagedown", kb), Some(109));
        assert_eq!(resolve_token("backspace", kb), Some(14));
        assert_eq!(resolve_token("delete", kb), Some(119));
        assert_eq!(resolve_token("f4", kb), Some(62));
        assert_eq!(resolve_token("0", kb), Some(11));
        assert_eq!(resolve_token("comma", kb), Some(51));
    }
}

#[test]
fn unknown_tokens_error() {
    for kb in [Kb::Windows, Kb::Apple] {
        assert_eq!(resolve_token("mousetrap", kb), None);
    }
}

#[test]
fn no_builtin_intercepts_ctrl_space() {
    use crate::map::{build_builtins, LauncherSpec};
    for kb in [Kb::Windows, Kb::Apple] {
        for ctx in [Ctx::Gui, Ctx::Terminal] {
            let launcher = LauncherSpec::Combo(crate::map::config::default_launcher());
            for r in build_builtins(kb, ctx, &launcher) {
                let ctrl_space = r.input.mods == vec![29] && r.input.key == Some(57);
                assert!(
                    !ctrl_space,
                    "builtin {} intercepts Ctrl+Space in ctx={ctx:?} — FORBIDDEN",
                    r.id
                );
            }
        }
    }
}

#[test]
fn compile_orders_users_first_and_honors_disable() {
    use crate::map::config::{default_launcher, Config, UserRule};
    let cfg = Config {
        general: crate::map::config::General {
            kbtype_default: "windows".into(),
            swap_caps_lctrl: true,
            adapter: "auto".into(),
            disable_builtins: vec!["cmd_h".into()],
            exclude_devices: vec![],
        },
        targets: crate::map::config::Targets { terminal_classes: vec![] },
        launcher: crate::map::config::Launcher {
            cmd_space: String::new(), // default Alt+Space
        },
        per_device: Default::default(),
        rules: vec![UserRule {
            input: "cmd+w".into(),
            output: "ignore".into(),
            context: "gui".into(),
            apps: None,
        }],
    };
    let gui = crate::map::compile(Kb::Windows, Ctx::Gui, &cfg);
    // user rule first
    assert_eq!(gui.rules[0].id, "user:cmd+w");
    // cmd_h disabled → no such rule anywhere
    assert!(!gui.rules.iter().any(|r| r.id == "cmd_h"));
    // cmd_space exists (launcher default)
    assert!(gui.rules.iter().any(|r| r.id == "cmd_space"));
}
