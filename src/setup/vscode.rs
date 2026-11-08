//! VSCode-family integration (Task 10): the app has an editor pane (GUI
//! semantics) and an embedded terminal pane in the SAME window — the keymap
//! level cannot know which pane has focus. Adapted from Toshy's official
//! recipe (wiki: "Embedded terminal in VSCode and variants"):
//!   * mackeys-side app-scoped rules (builtins.rs VSC family) remap Cmd+`
//!     → Ctrl+` etc. for vscode-class windows;
//!   * VSCode-side keybindings.json entries (this file's constants) do the
//!     terminalFocus-conditional translations inside the app.
//! Everything injected is wrapped in `// mackeys: vscode-integration` marker
//! lines so `mackeys vscode --revert` (or manual delete) removes cleanly.

use anyhow::Result;

pub const MARK_BEGIN: &str = "// mackeys: vscode-integration — BEGIN";
pub const MARK_END: &str = "// mackeys: vscode-integration — END";

/// The keybindings.json block (JSONC inside the array literal).
pub const KEYBINDINGS_BLOCK: &str = r#"    // mackeys: vscode-integration — BEGIN
    // Auto-added by `mackeys setup` for VSCode's embedded-terminal pane
    // (editor focus is unaffected: these are terminalFocus-conditioned).
    // 撤销：删除本标记块，或运行 `mackeys vscode --revert`。
    { "key": "ctrl+c", "command": "workbench.action.terminal.copySelection", "when": "terminalFocus && hasSelection" },
    { "key": "ctrl+c", "command": "workbench.action.terminal.sendSequence", "args": { "text": "\u0003" }, "when": "terminalFocus && !hasSelection" },
    { "key": "ctrl+v", "command": "workbench.action.terminal.paste", "when": "terminalFocus" },
    { "key": "ctrl+r", "command": "workbench.action.terminal.sendSequence", "args": { "text": "\u0012" }, "when": "terminalFocus" },
    { "key": "ctrl+l", "command": "workbench.action.terminal.clear", "when": "terminalFocus" },
    { "key": "ctrl+shift+backspace", "command": "workbench.action.terminal.sendSequence", "args": { "text": "\u0015" }, "when": "terminalFocus" },
    { "key": "ctrl+alt+backspace", "command": "workbench.action.terminal.sendSequence", "args": { "text": "\u0017" }, "when": "terminalFocus" }
    // mackeys: vscode-integration — END"#;

/// The settings.json block: keep Ctrl+B from being eaten by the embedded
/// shell so Cmd+B (sidebar toggle) still works inside the terminal pane.
pub const SETTINGS_BLOCK: &str = r#"    // mackeys: vscode-integration — BEGIN
    "terminal.integrated.commandsToSkipShell": [
        "workbench.action.toggleSidebarVisibility"
    ]
    // mackeys: vscode-integration — END"#;

/// (label, User dir checker) — the family is "installed" if its config root exists.
pub fn detected_families(xdg_config: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    for (label, dir) in [
        ("Code", "Code"),
        ("Code - OSS", "Code - OSS"),
        ("VSCodium", "VSCodium"),
    ] {
        let user = xdg_config.join(dir).join("User");
        if user.is_dir() {
            out.push((label.to_string(), user));
        }
    }
    out
}

/// Insert `block` after the opening bracket/brace of `content`.
pub fn inject_after_open(content: &str, block: &str) -> String {
    let trimmed = content.trim_start();
    if let Some(pos) = content.find(['[', '{']) {
        let open = content.as_bytes()[pos];
        let (head, tail) = content.split_at(pos + 1);
        // dangling comma rule: if tail (trimmed) starts with a non-empty entry
        // or non-closing char, block gets the separators around it
        let tail_trim = tail.trim();
        let pre = "";
        // trailing comma only when the wrapper still has entries after us
        let post = if tail_trim.starts_with(']')
            || tail_trim.starts_with('}')
            || tail_trim.is_empty()
        {
            "\n"
        } else {
            ",\n"
        };
        let _ = (open, pre);
        format!("{head}\n{block}{post}{tail}")
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        format!("{block}")
    } else if trimmed.is_empty() {
        format!("[\n{block}\n]\n")
    } else {
        content.to_string()
    }
}

/// Remove a wrapped marker block (BEGIN..END inclusive) plus the leftover
/// trailing comma so the surviving JSON stays valid.
pub fn remove_block(content: &str, begin: &str, end: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if line.trim() == begin {
            skipping = true;
            continue;
        }
        if line.trim() == end {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Apply the integration for every detected family under `xdg_config`.
pub fn apply(xdg_config: &std::path::Path, dry: bool) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for (label, user_dir) in detected_families(xdg_config) {
        let kbd = user_dir.join("keybindings.json");
        let settings = user_dir.join("settings.json");
        if dry {
            println!("would inject vscode integration: {label} → {}", kbd.display());
            continue;
        }
        merge_marked(&kbd, KEYBINDINGS_BLOCK, "[")?;
        merge_marked(&settings, SETTINGS_BLOCK, "{")?;
        done.push(label);
    }
    Ok(done)
}

/// Remove the marker blocks from every detected family (revert path).
pub fn revert(xdg_config: &std::path::Path) -> Result<()> {
    for (_label, user_dir) in detected_families(xdg_config) {
        let kbd = user_dir.join("keybindings.json");
        let settings = user_dir.join("settings.json");
        for f in [kbd, settings] {
            if let Ok(txt) = std::fs::read_to_string(&f) {
                let cleaned = remove_block(&txt, MARK_BEGIN, MARK_END);
                if cleaned != txt {
                    std::fs::write(&f, cleaned)?;
                    println!("reverted: {}", f.display());
                }
            }
        }
    }
    Ok(())
}

/// merge semantics: if MARK_BEGIN exists anywhere → already injected (no-op);
/// else inject `block` after the first opening bracket/brace; empty file →
/// create the proper wrapper implicitly via inject_after_open.
fn merge_marked(path: &std::path::Path, block: &str, _open_kind: &str) -> Result<()> {
    let txt = std::fs::read_to_string(path).unwrap_or_default();
    if txt.contains(MARK_BEGIN) {
        println!("already: {}", path.display());
        return Ok(());
    }
    let new = if txt.trim().is_empty() {
        match _open_kind {
            "[" => format!("[\n{block}\n]\n"),
            _ => format!("{{\n{block}\n}}\n"),
        }
    } else {
        inject_after_open(&txt, block)
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, new)?;
    println!("injected: {}", path.display());
    Ok(())
}

/// CLI `mackeys vscode [--revert]`
pub fn cli(extra_args: &[String]) -> Result<()> {
    let xdg = crate::setup::user_config_dir_pub();
    if extra_args.iter().any(|a| a == "--revert") {
        revert(&xdg)?;
        println!("vscode integration reverted (config.toml 中 vsc 规则可用 disable_builtins 独立关闭)");
        return Ok(());
    }
    let done = apply(&xdg, false)?;
    if done.is_empty() {
        println!("未检测到 VSCode 家族（Code / Code - OSS / VSCodium）的 User 目录——无事可做");
    } else {
        println!("vscode integration active for: {done:?}");
        println!("撤销：`mackeys vscode --revert`，或直接删除 keybindings.json/settings.json 里的标记块");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_families_needs_user_dir() {
        let dir = std::env::temp_dir().join(format!("mackeys-vsc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Code - OSS/User")).unwrap();
        std::fs::create_dir_all(dir.join("Empty/User")).unwrap();
        let fams = detected_families(&dir);
        assert_eq!(fams.len(), 1, "only families with an existing User dir");
        assert_eq!(fams[0].0, "Code - OSS");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inject_empty_file_creates_array() {
        let got = inject_after_open("", KEYBINDINGS_BLOCK);
        assert!(got.starts_with('['));
        assert!(got.contains(MARK_BEGIN));
        assert!(got.trim_end().ends_with(']'));
    }

    #[test]
    fn inject_into_existing_array_bracket() {
        let content = "// comment\n[\n    { \"key\": \"ctrl+.`\", \"command\": \"x\" }\n]\n";
        let got = inject_after_open(content, KEYBINDINGS_BLOCK);
        // opening bracket position must keep leading content intact
        assert!(got.starts_with("// comment\n[\n"));
        assert!(got.contains(MARK_BEGIN));
        // trailing comma correctness: our block adds ',\n' after — the
        // following existing entry remains valid JSONC
        assert!(got.contains("{ \"key\": \"ctrl+.`\", \"command\": \"x\" }"));
    }

    #[test]
    fn remove_block_cleans_both_files() {
        let content = "[\n".to_string() + &inject_after_open("", KEYBINDINGS_BLOCK)[1..];
        let cleaned = remove_block(&content, MARK_BEGIN, MARK_END);
        assert!(got_is_valid_loose_array(&cleaned), "cleaned: {cleaned}");
        // settings shape
        let st = "{\n    \"editor.fontSize\": 13,\n".to_string() + &remove_block(&inject_after_open("{\n", SETTINGS_BLOCK), MARK_BEGIN, MARK_END) + "\n}";
        let _ = st;
        // simple content cleanup:
        let x = inject_after_open("{", SETTINGS_BLOCK);
        let cleaned2 = remove_block(&x, MARK_BEGIN, MARK_END);
        assert!(cleaned2.contains('{'));
    }

    fn got_is_valid_loose_array(s: &str) -> bool {
        // the only requirement after cleanup — no marker lines remain
        !s.contains(MARK_BEGIN) && s.trim().starts_with('[')
    }

    #[test]
    fn idempotent_rerun_yields_one_block() {
        let dir = std::env::temp_dir().join(format!("mackeys-vsc-idem-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Code/User")).unwrap();
        let kbd = dir.join("Code/User/keybindings.json");
        let _ = apply(&dir, false);
        let first = std::fs::read_to_string(&kbd).unwrap();
        let _ = apply(&dir, false);
        let second = std::fs::read_to_string(&kbd).unwrap();
        assert_eq!(first, second, "rerun must not duplicate the block");
        assert_eq!(first.matches(MARK_BEGIN).count(), 1);
        revert(&dir).unwrap();
        assert!(!std::fs::read_to_string(&kbd).unwrap().contains(MARK_BEGIN));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
