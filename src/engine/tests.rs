//! Engine scenario tests (Task 2 of the core plan + later additions).
//! Pure: no evdev, no uinput — events in, commands out.

use super::{apply, code_map, shutdown, DeviceState};
use crate::types::{Act, Ctx, Combo, Kb, Out, OutputSel, Rule, RuleSet};

fn act(code: u16, value: i32) -> Act {
    Act { code, value }
}

fn combo(mods: &[u16], key: Option<u16>) -> Combo {
    Combo {
        mods: mods.to_vec(),
        key,
    }
}

/// Drive acts through a fresh state, collecting all Out commands.
fn drive(
    kb: Kb,
    swap: bool,
    ctx: Ctx,
    rules: &RuleSet,
    acts: &[Act],
) -> Vec<super::Out> {
    let mut st = DeviceState::default();
    let mut out = Vec::new();
    for a in acts {
        out.extend(apply(&mut st, *a, kb, swap, ctx, rules));
    }
    out
}

#[test]
fn shutdown_releases_tracked_and_ends_with_release_all() {
    let mut st = DeviceState::default();
    // hold cmd (physical 56), press+hold Z
    apply(&mut st, act(56, 1), Kb::Windows, true, Ctx::Gui, &RuleSet::default());
    apply(&mut st, act(44, 1), Kb::Windows, true, Ctx::Gui, &RuleSet::default());
    let out = shutdown(&mut st);
    assert!(out.contains(&Out::Key(44, 0)), "held combo key released");
    assert!(out.contains(&Out::Key(97, 0)), "held output mod released");
    assert!(matches!(out.last(), Some(Out::ReleaseAll)));
}

fn _wins_placeholder(ds: Ctx) -> Kb {
    Kb::Windows
}

fn rule_set(rules: Vec<Rule>) -> RuleSet {
    RuleSet { rules }
}

// ---- S1: passthrough_ctrl_space (Global Constraint: never intercepted) ----
#[test]
fn passthrough_ctrl_space() {
    // physical Caps (58) = the user's real Ctrl; combos with Space must fly
    // through as LEFT_CTRL(29)+SPACE(57) in every kbtype and context.
    let acts = [act(58, 1), act(57, 1), act(57, 0), act(58, 0)];
    let expected = [
        super::Out::Key(29, 1),
        super::Out::Sync,
        super::Out::Key(57, 1),
        super::Out::Sync,
        super::Out::Key(57, 0),
        super::Out::Sync,
        super::Out::Key(29, 0),
        super::Out::Sync,
    ];
    for kb in [Kb::Windows, Kb::Apple] {
        for ctx in [Ctx::Gui, Ctx::Terminal] {
            let got = drive(kb, true, ctx, &RuleSet::default(), &acts);
            assert_eq!(
                got, expected,
                "kbtype={kb:?} ctx={ctx:?} — Ctrl+Space must pass through"
            );
        }
    }
}

// ---- S2: swap_caps_base (windows: caps->lctrl, lctrl->caps, cmd->r_ctrl) ----
#[test]
fn swap_caps_base_windows() {
    let kb = Kb::Windows;
    let rs = rule_set(Vec::new());
    // physical 左Ctrl (29) pressed => output is CAPSLOCK (58)
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(29, 1)]);
    assert_eq!(got.len(), 2, "one Key + one Sync");
    assert!(got.contains(&super::Out::Key(58, 1)));
    // physical Caps (58) => output LEFT_CTRL (29)
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(58, 1)]);
    assert!(got.contains(&super::Out::Key(29, 1)));
    // physical 左Alt (56) = Cmd => output RIGHT_CTRL (97)
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(56, 1)]);
    assert!(got.contains(&super::Out::Key(97, 1)));
    // physical 左Win (125) = Opt => output LEFT_ALT (56)
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(125, 1)]);
    assert!(got.contains(&super::Out::Key(56, 1)));
}

#[test]
fn swap_caps_base_apple() {
    let kb = Kb::Apple;
    let rs = rule_set(Vec::new());
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(58, 1)]);
    assert!(got.contains(&super::Out::Key(29, 1)));
    // Apple: physical 左Meta(125)=Cmd => RIGHT_CTRL(97)
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(125, 1)]);
    assert!(got.contains(&super::Out::Key(97, 1)));
    // Apple: physical 左Alt(56)=Opt => passthrough LEFT_ALT(56)
    let got = drive(kb, true, Ctx::Gui, &rs, &[act(56, 1)]);
    assert!(got.contains(&super::Out::Key(56, 1)));
}

// ---- S3: repeats (value==2) dropped globally ----
#[test]
fn repeats_dropped() {
    for kb in [Kb::Windows, Kb::Apple] {
        let got = {
            let mut st = DeviceState::default();
            apply(
                &mut st,
                act(57, 2), // space repeat
                kb,
                true,
                Ctx::Gui,
                &RuleSet::default(),
            )
        };
        assert!(got.is_empty(), "value=2 events must vanish");
        let got = {
            let mut st = DeviceState::default();
            apply(
                &mut st,
                act(58, 2), // caps repeat
                kb,
                true,
                Ctx::Gui,
                &RuleSet::default(),
            )
        };
        assert!(got.is_empty(), "modifier repeats must vanish too");
    }
}

// ---- S4: combo rewrite (GUI letter family via modmap-held output mods) ----
#[test]
fn gui_family_rewrite_shows_option_scan() {
    // cmd+Z → ctrl+Z: rule input (held R_CTRL=97 from cmd, key Z=44) simply
    // flows through the modmap: the exercised path is that held cmd yields
    // R_CTRL on output and the letter key re-emits unchanged.
    let acts = [
        act(56, 1),  // physical lalt (cmd) down -> R_CTRL down
        act(44, 1),  // Z down
        act(44, 0),  // Z up
        act(56, 0),  // cmd up
    ];
    let expected = [
        super::Out::Key(97, 1),
        super::Out::Sync,
        super::Out::Key(44, 1),
        super::Out::Sync,
        super::Out::Key(44, 0),
        super::Out::Sync,
        super::Out::Key(97, 0),
        super::Out::Sync,
    ];
    let got = drive(Kb::Windows, true, Ctx::Gui, &RuleSet::default(), &acts);
    assert_eq!(got, expected);
}

// ---- S5: Ignore swallows the key and releases own output mods ----
#[test]
fn ignore_swallows_and_releases() {
    // rule: R_CTRL(97) + H(35) -> Ignore
    let rs = rule_set(vec![Rule {
        id: "cmd_h".into(),
        builtin: true,
        sticky: false,
        context: Ctx::Gui,
        input: combo(&[97], Some(35)),
        output: OutputSel::Ignore,
    }]);
    let acts = [
        act(56, 1), // cmd down -> R_CTRL(97) down
        act(35, 1), // H down  -> swallowed, 97 released
        act(35, 0), // H up    -> still nothing (already released cleanly)
        act(56, 0), // cmd up  -> nothing (97 no longer claimed)
    ];
    let got = drive(Kb::Windows, true, Ctx::Gui, &rs, &acts);
    // allowed stream: 97 down, H swallow w/ 97 release, then nothing further
    assert_eq!(
        got,
        vec![
            super::Out::Key(97, 1),
            super::Out::Sync,
            super::Out::Key(97, 0),
            super::Out::Sync,
        ],
        "Cmd+H must be swallowed (no Key(35,...)) and own mods released exactly once"
    );
    assert!(!got.contains(&super::Out::Key(35, 1)));
    assert!(!got.contains(&super::Out::Key(35, 0)));
}

// ---- S6: combo re-press after auto-release (resume semantics) ----
#[test]
fn resume_after_combo_release() {
    // cmd+X then cmd+Y while cmd held: after X release the output ctrl may be
    // released; Y press must re-establish it before the letter.
    let acts = [
        act(56, 1), // cmd down -> 97 down
        act(45, 1), // X down  (no rule: passthrough w/ held 97)
        act(45, 0), // X up
        act(44, 1), // Z down
        act(44, 0),
        act(56, 0), // cmd up
    ];
    let got = drive(Kb::Windows, true, Ctx::Gui, &RuleSet::default(), &acts);
    // find index of last 45 release and first 44 press: between them nothing,
    // and 44 press must be preceded by 97 press (re-resume) repeating.
    let mut saw_after_x = Vec::new();
    let mut it = got.iter().peekable();
    for ev in got.iter() {
        match ev {
            super::Out::Key(45, 0) => break, // stop after X release
            _ => {}
        }
        saw_after_x.push(*ev);
        let _ = it.next();
    }
    // collect stream between X-release and Z-press
    let mut z_press_idx = got
        .iter()
        .position(|e| matches!(e, super::Out::Key(44, 1)))
        .expect("Z press present");
    let x_rel_idx = got
        .iter()
        .position(|e| matches!(e, super::Out::Key(45, 0)))
        .expect("X release present");
    let between = &got[x_rel_idx + 1..z_press_idx];
    // ??? — passthrough semantics may keep 97 down (never released) instead of
    // auto-release — either way Z press while cmd held must see ctrl held.
    // The contract we pin here: Z press event is present and 97 has been
    // re-asserted if it was dropped between X and Z.
    let _ = between; // inspected by explicit second check below
    z_press_idx += 0;
    // passthrough must NOT have auto-released 97 between X and Z:
    assert!(
        !got[x_rel_idx..z_press_idx]
            .iter()
            .any(|e| matches!(e, super::Out::Key(97, 0))),
        "passthrough keeps held mods down — no spurious 97 release between keys"
    );
}

// ---- S7: held combo output shape with a simple rewrite rule ----
#[test]
fn combo_rule_stream_shape() {
    // rule: R_CTRL(97)+C(46) -> output R_CTRL... only to verify held-mod
    // dedup: output mods that are already pressed are not re-pressed.
    let rs = rule_set(vec![Rule {
        id: "keep_c".into(),
        builtin: false,
        sticky: false,
        context: Ctx::Gui,
        input: combo(&[97], Some(46)),
        output: OutputSel::Combo(combo(&[97], Some(46))),
    }]);
    let acts = [
        act(56, 1), // cmd -> 97
        act(46, 1), // C down: output combo mods [97] already held -> only Key(46,1)
        act(46, 0),
        act(56, 0),
    ];
    let got = drive(Kb::Windows, true, Ctx::Gui, &rs, &acts);
    // must NOT contain a second 97 press on C press
    let press97: Vec<_> = got.iter().filter(|e| matches!(e, super::Out::Key(97, 1))).collect();
    assert_eq!(press97.len(), 1, "R_CTRL pressed exactly once");
    let pressC = got.iter().filter(|e| matches!(e, super::Out::Key(46, 1))).count();
    assert_eq!(pressC, 1);
}
