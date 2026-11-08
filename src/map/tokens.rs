//! Token DSL: rule-side tokens resolve to POST-modmap output codes
//! (T2 ruling): cmd→R_CTRL, ctrl/caps→L_CTRL, opt/alt/meta→L_ALT.
//! Same values for all kbtypes; only the physical origin differs.

use crate::types::Kb;

pub fn resolve_token(token: &str, _kb: Kb) -> Option<u16> {
    Some(match token {
        // aliases → output-side codes
        "cmd" | "rcmd" => crate::engine::R_CTRL,
        "ctrl" | "lctrl" | "caps" => crate::engine::L_CTRL,
        "opt" | "alt" | "meta" | "lopt" | "lalt" | "lmeta" => crate::engine::L_ALT,
        "shift" => 42,

        // non-modifiers: physical identity (ANSI assumption)
        "space" => 57,
        "tab" => 15,
        "grave" => 41,
        "enter" => 28,
        "esc" => 1,
        "backspace" => 14,
        "delete" => 119,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "insert" => 110,
        "minus" => 12,
        "equal" => 13,
        "comma" => 51,
        "dot" => 52,
        "slash" => 53,
        "semicolon" => 39,
        "apostrophe" => 40,
        "bracketleft" => 26,
        "bracketright" => 27,
        "backslash" => 43,
        "0" => 11,
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        t if t.len() == 1 && t.as_bytes()[0].is_ascii_alphabetic() => {
            // a..z → evdev letter codes (QWERTY physical order)
            const LETTERS: [u16; 26] = [
                30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20,
                22, 47, 17, 45, 21, 44,
            ]; // a b c d e f g h i j k l m n o p q r s t u v w x y z
            LETTERS[t.as_bytes()[0] as usize - b'a' as usize]
        }
        _ => return None,
    })
}
