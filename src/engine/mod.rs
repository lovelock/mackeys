//! Pure keymap engine: `Act -> Vec<Out>`. No evdev, no uinput — fully unit
//! testable. Semantics pinned by `engine/tests.rs` and the core plan.

use crate::types::{Act, Combo, Ctx, Kb, Out, OutputSel, Rule, RuleSet};

pub const L_CTRL: u16 = 29;
pub const L_ALT: u16 = 56;
pub const L_META: u16 = 125;
pub const CAPSLOCK: u16 = 58;
pub const R_CTRL: u16 = 97;
pub const R_ALT: u16 = 100;
pub const R_META: u16 = 126;

/// Physically-held modifier mapping input physical code -> output code.
#[derive(Debug, Default)]
struct Keyst {
    in_code: u16,
    out_code: u16,
}

#[derive(Debug)]
struct HeldCombo {
    key_in: u16,
    #[allow(dead_code)]
    out: Combo,
    out_pressed: Vec<u16>,
}

/// Per-device state (spec: never share across devices).
#[derive(Debug, Default)]
pub struct DeviceState {
    held: Vec<Keyst>,
    auto_released: Vec<u16>,
    bound: Vec<(u16, u16)>,
    held_combo: Option<HeldCombo>,
    /// If a rule with Ignore matched, its key's release is swallowed too
    /// (until the same key releases, or any other event happens).
    pending_ignored: Option<u16>,
    /// Output-side keys currently believed pressed (down) — incl. passthrough.
    down_keys: Vec<u16>,
}

pub fn is_modifier(code: u16) -> bool {
    matches!(
        code,
        L_CTRL | L_ALT | L_META | CAPSLOCK | R_CTRL | R_ALT | R_META
    )
}

/// OB code translator. windows: caps<->lctrl, lalt->r_ctrl, lmeta->l_alt.
/// apple: caps<->lctrl, lmeta->r_ctrl, lalt passthrough.
pub fn code_map(kb: Kb, swap_caps: bool) -> impl Fn(u16) -> u16 + 'static {
    move |code| match code {
        58 if swap_caps => L_CTRL,
        29 if swap_caps => CAPSLOCK,
        56 if kb == Kb::Windows => R_CTRL,
        125 if kb == Kb::Windows => L_ALT,
        125 if kb == Kb::Apple => R_CTRL,
        other => other,
    }
}

impl DeviceState {
    fn held_outs(&self) -> Vec<u16> {
        let rel = &self.auto_released;
        self.held
            .iter()
            .map(|k| k.out_code)
            .filter(|o| !rel.contains(o))
            .collect()
    }

    fn stop_combo(&mut self) -> Vec<Out> {
        let Some(h) = self.held_combo.take() else {
            return vec![];
        };
        let mut outv = vec![Out::Key(h.key_in, 0)];
        for m in &h.out_pressed {
            outv.push(Out::Key(*m, 0));
            if let Some(ks) = self
                .held
                .iter()
                .find(|k| k.out_code == *m)
                .map(|k| k.out_code)
            {
                if !self.auto_released.contains(&ks) {
                    self.auto_released.push(ks);
                }
            }
            self.bound.retain(|(_, o)| o != m);
        }
        outv.push(Out::Sync);
        outv
    }

    fn mark_auto_released(&mut self, out_code: u16) {
        if !self.auto_released.contains(&out_code) {
            self.auto_released.push(out_code);
        }
    }
}

/// Process one input act, returning output commands.
fn process_inner(
    state: &mut DeviceState,
    act: Act,
    kb: Kb,
    swap_caps: bool,
    ctx: Ctx,
    rules: &RuleSet,
) -> Vec<Out> {
    let km = code_map(kb, swap_caps);
    let out_code = km(act.code);

    if is_modifier(out_code) && act.value != 2 {
        return match act.value {
            1 => {
                if let Some(pos) = state.held.iter().position(|k| k.in_code == act.code) {
                    let old = state.held.remove(pos);
                    if old.out_code != out_code {
                        // remap changed identity mid-hold — release old shape
                        state.auto_released.retain(|c| *c != old.out_code);
                    }
                }
                state.held.push(Keyst {
                    in_code: act.code,
                    out_code,
                });
                vec![Out::Key(out_code, 1), Out::Sync]
            }
            _ => {
                if !state
                    .held
                    .iter()
                    .any(|k| k.in_code == act.code && k.out_code == out_code)
                {
                    // stale release passthrough
                    return vec![Out::Key(out_code, 0), Out::Sync];
                }
                state
                    .held
                    .retain(|k| !(k.in_code == act.code && k.out_code == out_code));
                let was_released = state.auto_released.contains(&out_code);
                state.auto_released.retain(|c| *c != out_code);
                if was_released {
                    // already released silently (Ignore / combo teardown) — stay silent
                    return vec![];
                }
                vec![Out::Key(out_code, 0), Out::Sync]
            }
        };
    }

    // ---- non-modifier ----
    match act.value {
        1 => {
            // a key press while a combo is held: teardown then proceed
            let teardown = state.stop_combo();
            let mut outv = teardown; // (rare path; keeps output sane)
            state.pending_ignored = None;

            let held_outs = state.held_outs();
            let matched = rules
                .rules
                .iter()
                .find(|r| r.context == ctx && matches_combo(&r.input, &held_outs, out_code));

            match matched.map(|r| (&r.output, r.id.as_str())) {
                Some((OutputSel::Ignore, _)) => {
                    // swallow + release own output mods (they stay auto-released
                    // so the later physical release is silent)
                    state.pending_ignored = Some(out_code);
                    if let Some(r) = rules
                        .rules
                        .iter()
                        .find(|r| r.context == ctx && matches_combo(&r.input, &held_outs, out_code))
                    {
                        for m in &r.input.mods {
                            if held_outs.contains(m) {
                                outv.push(Out::Key(*m, 0));
                                state.mark_auto_released(*m);
                            }
                        }
                    }
                    outv.push(Out::Sync);
                    outv
                }
                Some((OutputSel::Combo(c), _)) => {
                    let mut pressed = Vec::new();
                    for m in &c.mods {
                        if state
                            .held
                            .iter()
                            .any(|k| k.out_code == *m)
                            && !state.auto_released.contains(m)
                            && !pressed.contains(m)
                        {
                            continue; // already down
                        }
                        if pressed.contains(m) {
                            continue;
                        }
                        outv.push(Out::Key(*m, 1));
                        pressed.push(*m);
                    }
                    outv.push(Out::Key(out_code, 1));
                    outv.push(Out::Sync);
                    state.held_combo = Some(HeldCombo {
                        key_in: out_code,
                        out: c.clone(),
                        out_pressed: pressed,
                    });
                    outv
                }
                Some((OutputSel::Pass, _)) | None => {
                    // resume auto-released mods, then emit the key
                    for m in std::mem::take(&mut state.auto_released) {
                        outv.push(Out::Key(m, 1));
                    }
                    outv.push(Out::Key(out_code, 1));
                    outv.push(Out::Sync);
                    outv
                }
            }
        }
        _ => {
            // a swallowed key's release is swallowed too
            if state.pending_ignored == Some(out_code) {
                state.pending_ignored = None;
                return vec![];
            }
            state.pending_ignored = None;
            if let Some(h) = &state.held_combo {
                if h.key_in == out_code {
                    let tail = state.stop_combo();
                    return tail;
                }
            }
            vec![Out::Key(out_code, 0), Out::Sync]
        }
    }
}

fn matches_combo(input: &Combo, held_outs: &[u16], key: u16) -> bool {
    input.key == Some(key) && input.mods.iter().all(|m| held_outs.contains(m))
}

/// Release every pressed output key/canonical mod (termination path).
pub fn shutdown(state: &mut DeviceState) -> Vec<Out> {
    let mut outv = state.stop_combo();
    let mut released: Vec<u16> = Vec::new();
    for k in state.held.iter() {
        if state.auto_released.contains(&k.out_code) || released.contains(&k.out_code) {
            continue;
        }
        outv.push(Out::Key(k.out_code, 0));
        released.push(k.out_code);
    }
    for b in std::mem::take(&mut state.bound) {
        if released.contains(&b.1) {
            continue;
        }
        outv.push(Out::Key(b.1, 0));
        released.push(b.1);
    }
    for d in std::mem::take(&mut state.down_keys) {
        if released.contains(&d) {
            continue;
        }
        outv.push(Out::Key(d, 0));
        released.push(d);
    }
    outv.push(Out::ReleaseAll);
    state.auto_released.clear();
    state.held.clear();
    outv
}

/// Release "own" output mods after an Ignore (kept pub for later reuse).
pub fn release_ignored(state: &mut DeviceState, mods: &[u16]) -> Vec<Out> {
    let mut outv = Vec::new();
    for m in mods {
        if state.held.iter().any(|k| k.out_code == *m)
            && !state.auto_released.contains(m)
        {
            outv.push(Out::Key(*m, 0));
            state.mark_auto_released(*m);
        }
    }
    outv.push(Out::Sync);
    outv
}

/// Public entry: drops kernel-autorepeat events, tracks pressed output keys.
pub fn apply(
    state: &mut DeviceState,
    act: Act,
    kb: Kb,
    swap_caps: bool,
    ctx: Ctx,
    rules: &RuleSet,
) -> Vec<Out> {
    if act.value == 2 {
        return vec![]; // kernel autorepeat — compositor drives repeat (spec §8)
    }
    let outv = process_inner(state, act, kb, swap_caps, ctx, rules);
    for out in &outv {
        match out {
            Out::Key(c, 1) => {
                if !state.down_keys.contains(c) {
                    state.down_keys.push(*c);
                }
            }
            Out::Key(c, 0) => {
                if let Some(pos) = state.down_keys.iter().position(|d| d == c) {
                    state.down_keys.remove(pos);
                }
            }
            _ => {}
        }
    }
    outv
}

#[cfg(test)]
mod tests;
