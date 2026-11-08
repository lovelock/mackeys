//! Input/output value types shared by the pure engine, the device pipe and
//! the rule compiler. Device-free: everything here is testable in isolation.

/// One evdev EV_KEY event, device-agnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Act {
    pub code: u16,
    pub value: i32, // 1=down, 0=up, 2=repeat(dropped globally, see spec §8)
}

/// Commands for the uinput sink. The sink decides SYN_REPORT grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Key(u16, i32),
    Sync,
    /// Release every tracked output key (termination path only).
    ReleaseAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    Gui,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kb {
    Windows,
    Apple,
}

/// A combo: held output-side modifier codes + an (optional) key, all physical
/// evdev codes. Output-side = no role aliases (spec §4 FocusSource; tokens.rs
/// resolves aliases on the INPUT side at compile time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    pub mods: Vec<u16>,
    pub key: Option<u16>,
}

/// How a matched rule responds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputSel {
    Combo(Combo),
    /// Swallow the key entirely (e.g. Cmd+H).
    Ignore,
    /// Matches but emits nothing special — identical to no rule matching.
    Pass,
}

/// One rule. Semantics: first-match-wins across `[rules]` ordering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub builtin: bool,
    pub sticky: bool,
    pub context: Ctx,
    pub input: Combo,
    pub output: OutputSel,
}

/// Ordered rule set; index 0 has the highest priority.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}
