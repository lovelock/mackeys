# mackeys Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Single Rust binary `mackeys` — an evdev→uinput Wayland keymapper that applies the owner's macOS keymap (per spec §3) with a daemon + setup/doctor tooling, no GUI.

**Architecture:** A pure, device-free engine ("`InputEvent → Vec<Out>`") is the core; around it: per-device blocking evdev reader threads feeding a channel, one engine consumer, an `evdev::VirtualDeviceBuilder` sink, a zbus adapter for KWin focus context, and a `setup/doctor` CLI. Built-in keymaps compile per kbtype from a token DSL; idempotent setup discipline mirrors the hhkb preset work.

**Tech Stack:** Rust (edition 2024), `evdev` (input grab + `VirtualDeviceBuilder` output), `zbus` (dbus), `serde`+`toml`, `tracing`, `anyhow`; OS threads + channel (no tokio needed).

**Spec:** `docs/superpowers/specs/2026-11-08-mackeys-design.md` (binding authority; §8 round-2 decisions govern over §3 draft wording where they conflict).

## Global Constraints

- All rules match on **physical evdev codes**; layout-independent; ANSI assumption.
- `Ctrl+Space` is **never** matched by any builtin rule in any context (spec §3 + SPOTL lesson).
- Terminal `Cmd+C` output is `Ctrl+Shift+C`, never plain `Ctrl+C` (SIGINT safety).
- Engine never synthesizes autorepeat: input `value=2` dropped globally; held output keys get compositor-driven repeat (spec §8).
- evdev constants pinned: RC=29(R_CTRL 97), LA=56(左Alt), LM=125, CAPS=58, LCTL=29, RCTL=97, RALT=100, RMETA=126, TAB=15, GRAVE=41, SPACE=57, C=46, V=47, X=45, Z=44, W=17, T=20, N=49, H=35, Q=16, BACKSPACE=14, DELETE=119, LEFT=105, RIGHT=106, UP=103, DOWN=108, HOME=102, END=107, PAGEUP=104, PAGEDOWN=109, ESC=1, F1..F7=59..65, ENTER=28, MINUS=12, EQUAL=13, COMMA=51, DOT=52, KEY0..KEY9=11..2 (数字0=11).
- kbtype v1: `windows` | `apple` only; anything else errors at load.
- Virtual device name: `mackeys virtual keyboard`.
- `setup` writes ONLY inside `$HOME/.config/mackeys`, systemd user units, `~/.local/share/kwin/scripts/`; udev = print-instructions-only.

## Review Focus

1. Terminal `Cmd+C` must yield `Ctrl+Shift+C` — engine test `term_cmd_c`.
2. `Ctrl+Space` passthrough in every context/kbtype — engine test `passthrough_ctrl_space`.
3. `Cmd+Tab` double-tap cycles forward per tap (sticky Alt held; release commits) — engine test `tab_cycle`.
4. Output mods leak after teardown — engine test `teardown_releases`.
5. `setup` rerun must not duplicate writes — test `setup_idempotent`.

---

### Task 1: Repo scaffold + project shape

**Files:** Create `Cargo.toml`, `src/main.rs` (clap-less arg dispatch: subcommand strings), `src/types.rs`, `src/engine/mod.rs`(empty), `src/map/mod.rs`(empty), `src/adapters/mod.rs`(FocusSource trait + 注册表骨架，impl 首版空形), `src/setup/mod.rs`(empty), `README.md`(占位), `rust-toolchain.toml`(stable), `LICENSE`(MIT), `docs/superpowers/{plans,specs}/…`（已有）.

**Interfaces:** Produces binary skeleton `mackeys <subcommand>` returning unimplemented errors for later subcommands; envs: none.

- [ ] Step 1: `git init` + cargo init + deps(direct)：`evdev`, `zbus`(feature blocks不可用缺省), `serde`+`toml`, `anyhow`, `tracing`+`tracing-subscriber`; dev-deps: 无（自带测试跑件）。
- [ ] Step 2: `src/main.rs` 的 `match args[1]` 分发 + `todo!()`占位；`cargo build` 成功即通过锚点。
- [ ] Step 3: `gh repo create lovelock/mackeys --public --source . --push`（私有与否后续 owner 可 `gh repo edit --visibility`）。
- [ ] Step 4: commit `chore: scaffold mackeys project`。

### Task 2: types + pure passthrough/modmap engine（无设备依赖、纯函数）

**Files:** Create `src/types.rs`(追加), `src/engine/mod.rs`(实现), `src/engine/tests.rs`(或 `#[cfg(test)]` 块)。

**Interfaces:**
- Consumes: 无外部。
- Produces（全部 pub，后续任务复用，不得重命名）:
  - `pub struct Act { pub code: u16, pub value: i32 }` （输入）
  - `pub enum Out { Key(u16, i32), Sync, ReleaseAll }`（输出命令；Key=EV_KEY+value，Sync=SYN_REPORT 时机由 sink 决定：每条 Out::Key 立即 Sync 也行——Sink trait 决策）
  - `pub enum Ctx { Gui, Terminal }`
  - `pub enum Kb { Windows, Apple }`
  - `pub struct Combo { pub mods: Vec<u16>, pub key: Option<u16> }`
  - `pub enum OutputSel { Combo(Combo), Ignore, Pass }`
  - `pub struct Rule { pub id: String, pub builtin: bool, pub sticky: bool, pub context: Ctx, pub input: Combo, pub output: OutputSel }`
  - `pub struct RuleSet { pub rules: Vec<Rule> }`（顺序 == 优先级，**第一个匹配赢**）
  - `pub fn code_map(kb: Kb, swap_caps: bool) -> impl Fn(u16) -> u16 + 'static`：windows → `58→29, 29→58, 56→97, 125→56`；apple → `56→97, 125→56` + 同样的 58↔29
  - `pub struct DeviceState { ... }`（内部态：output-side held mods `Vec<(in_code,out_code)>`、bound/sticky 对 `Vec<(in,out)>`、held combo `Option<(Combo, 输入触键)>`）
  - `pub fn apply(state: &mut DeviceState, act: Act, kb: Kb, swap_caps: bool, ctx: Ctx, rules: &RuleSet) -> Vec<Out>`
  - `pub fn shutdown(state: &mut DeviceState) -> Vec<Out>`（先把全部 in-bound bind 对构成 output 释放，后 ReleaseAll）
  - `pub fn release_ignored(state: &mut DeviceState) -> Vec<Out>`（Ignore 触发时用于释放 own-output mods）

**行为规格（测试钉死）**：
- `value == 2` 的 act：全局丢弃，返回 `[]`。
- modmap 每条 act 的 code 先过 `code_map(kb, swap)`；win键位下 caps(58)→29、29→58。
- 非修饰键且规则未命中 → 输出 `Key(mapped.code, value)` 和 `Sync`。
- 键盘 when 两条会话内死条规则：规则匹配以**输出侧 mods**(在按) + 映射后 inkey 组成 combo 与 rule.input 比较；`Ignore` 无输出（吞键）并释放 own output mods；`Pass` 等价未命中。
- `Combo` 输出：按下输入键时刻 → 释放 own output mods（输出 combo 中不再持有的）→ `Key(输出mods,1)`×n → `Key(key,1)` → `Sync`；输入键 release → `Key(key,0)` → 再按每个输出 mod `Key(mod,0)`——**除非** sticky(该 mod 来自绑定输入修饰键仍然按住)——§详见 sticky（Task 6 再落，Task 2 先实现非 sticky 版）。

- [ ] Step 1: RED——写测试：`passthrough_ctrl_space`（两种 kbtype × 两种 ctx 下 `(29,1),(57,1),(57,0),(29,0)` 输出恒等于原 sequence+Sync 恒等于映射后同 code——这里 29→29/29→58 apple?(apple 29→? windows 29→58)——键盘 windows 中 29→58 是输出 caps?? 在 windows 表中左ctrl→caps，则按下 Caps的 code 实际是 58 —— 设计文件测试写清：物理码按 windows 表后测试期望；(b) input 29 (左Ctrl物理) windows 映射后=58 输出 Key(58,1)）→ 失败（未实现）。
- [ ] Step 2: GREEN——实现 `code_map`+`apply`（无规则行为：空 RuleSet=全透+modmap）。
- [ ] Step 3: RED——测试 `swap_caps_base`：windows 下具体输入 58/29 的方向断言；`OutputSel::Ignore` 吞键测试；`Combo` 基础输出测试（e.g. rule cmd_c：input{mods:[56],key:46}(windows) output Combo{mods:[97],key:46} → (56,1)(46,1)(46,0)(56,0) 应输出 (97,1)(46,1)(46,0)(97,0)）→ 失败。
- [ ] Step 4: GREEN——补齐 apply 的规则判定与输出。
- [ ] Step 5: `cargo test` 全绿；commit `feat(engine): pure keymap engine with modmap + rules`。

### Task 3: 真实设备管道（evdev 抓取 + uinput 输出 + 生命周期）

**Files:** Create `src/daemon/mod.rs`, `src/daemon/input.rs`, `src/daemon/sink.rs`; Modify `src/main.rs`（`run` 子命令接通）。

**Interfaces:**
- Consumes: Task 2 的 `apply/shutdown`、`Out/Act/Ctx/Kb`。
- Produces:
  - `pub fn spawn(self: &Daemon) -> anyhow::Result<JoinHandle<()>>`：枚举 `/dev/input/event*`，过滤（见下），每设备 spawn 阻塞读线程→`mpsc::Sender<DevEv{tag, act}>`；uinput sink 用 `evdev::VirtualDeviceBuilder` 建 `mackeys virtual keyboard`（带才EV_KEY 全键域 + LED 位），主循环 `recv()->DevEv` → `apply(...)` → 逐条 `Out` 发给 sink。
  - 设备过滤：含字母键（`KeyCode` 域有 46）才纳管；跳过 name=="mackeys virtual keyboard"；`[per_device] exclude_devices`。
  - 热插拔：goroutine 监听 udev netlink（或轮询 `/proc/bus/input/devices` mtime）add/remove → 对应 reader 重建/终止（graphical-session suspend/resume 复用同路径）。
  - 取消/退出组合：ctrl-c 与 SIGTERM → 每 device `apply(shutdown)` 输出 flush → device.ungrab() → 退出（Toshy Output.shutdown 等价）。
  - SYN_DROPPED：evdev crate 自动 resync，但 reader 必须在 resync 后用 `device.get_key_state_v2()` 重放 keystate（多按住的键恢复为输出按下态），防止修饰键泄漏。
  - grab 起始同样以 `get_key_state_v2()` 同步初始按住态（Toshy keystate 同步教训）。
- Token `Ctx` 本任务恒 `Ctx::Gui`（KWin 适配器 Task 7 接）。

- [ ] Step 1: 手工冒烟协议（不可自动测试真实 grab）——写 `mackeys run --dry`：仅枚举并打印纳管设备清单（供 `devices` 复用），配 assert 读名单逻辑单测 `filter_letter_devices`。
- [ ] Step 2: 实现管道（含热插拔管理循环）+ `run`；`cargo run -- run --dry` 输出含 "BY Tech Gaming Keyboard"。
- [ ] Step 3: 手工验证清单（打印到 output，owner 后续按清单验收）：真实 take over、按键透传一致性、污染综合（修饰按住时另一设备敲键不串扰——本任务只要求代码路 wired，逐条验证放 Task 10）。
- [ ] Step 4: `cargo build --release`；commit `feat(daemon): evdev pipe + uinput sink + lifecycle`。

### Task 4: map 编译器 + 内置 GUI 表 + 配置加载 + `dump`

**Files:** Create `src/map/mod.rs`, `src/map/tokens.rs`, `src/map/builtins.rs`, `src/map/config.rs`; Modify `src/main.rs`（`dump` 子命令）。

**Interfaces:**
- Consumes: Task 2 的 `Rule/RuleSet/Combo/OutputSel/Ctx/Kb`、evdev 常量。
- Produces:
  - `pub struct BuiltinId(pub &'static str)`：具体常量 `CMD_TAB, CMD_SHIFT_TAB, CMD_GRAVE, CMD_SPACE, CMD_H, CMD_Q, CMD_ARROW_HN, CMD_WORDWISE, TERM_CMD_C, TERM_CMD_FAMILY, TERM_CMD_ARROWS, TERM_CMD_NUMS, CMD_COMMA…`（表格见 spec §3.1/3.2 的表；id 集合在 builtins.rs 以常量数组恒定）
  - `pub fn build_builtins(kb: Kb, launcher: &LauncherSpec) -> Vec<Rule>`（LauncherSpec::AltSpace | MetaSpace | None；None → CMD_SPACE 生成 Ignore——记住 spec §8：无=吞键）
  - `pub fn load_config(path) -> Config`；`Config::validate()`（未知键报错；kbtype v1 refuse ibm/chromebook）
  - `pub fn compile(kb, contig: &Config) -> RuleSet`：disable_builtins 排除 id → 用户 `[[rules]]` 放最前 → 内置规则
  - `pub fn dump(sets: &BTreeMap<Kb, RuleSet>) -> String`：编译后规则按 first-match 顺序文本化
- Token（`tokens.rs`）：`"cmd","opt","alt","meta","ctrl","caps","win","lalt","ralt"|"space","tab","grave","comma","dot","minus","equal","enter","esc","home","end","pageup","pagedown","delete","backspace","up","down","left","right"|"a..z","0..9","f1..f7"` → evdev code；**cmd/opt 是角色**（windows: cmd=lalt, opt=lmeta；apple: cmd=lmeta, opt=lalt）——输入侧规则用角色 token；输出侧 Combo 直接是物理 evdev code（无角色）。

- [ ] Step 1: RED——parse 单测：`parse_token("cmd", Windows) == 56`；`parse_token("cmd", Apple) == 125`；未知 token 报错；`parse_combo_str("cmd+c", kb)`。
- [ ] Step 2: GREEN——实现 tokens.rs。
- [ ] Step 3: RED——`builtins_windows_set` 断言：CMD_SPACE.input=={mods:[56],key:57}、output==Combo{[56(lalt→code56),57]}……并检查任何 builtin 的 input 都 ≠ {mods:[29],key:57}（Global Constraint 的“永不拦截 Ctrl+Space”）。
- [ ] Step 4: GREEN——builtins.rs 全表。
- [ ] Step 5: RED——`compile` 顺序测试：disable 移除对应 id；用户规则排最前。
- [ ] Step 6: GREEN——compile + load_config。
- [ ] Step 7: `cargo test` 全绿；commit `feat(map): token dsl, builtins, config, dump`。

### Task 5: 终端上下文全家桶 + 上下文门控

**Files:** Modify `src/map/builtins.rs`（终端表）、`src/engine/mod.rs`（行为测试只读）；测试位 `src/engine/tests.rs`。

**Interfaces:**
- Consumes: Task 2 apply/设备态、Task 4 编译器。
- Produces: `pub fn active_ctx(user_set: Option<Ctx>, kwin_report: Option<Ctx>) -> Ctx`（规则：display 可超控，默认无 report = Gui）。

- [ ] Step 1: RED——`term_cmd_c`：windows 下 device 收 `(56,1)(46,1)(46,0)(56,0)`、ctx=Terminal → 输出 `(97,1)(42,1)(46,1)(46,0)(42,0)(97,0)` 中含 LSHIFT=42；**必须** 断言Shift 在 C 之前按下/最后释放。
- [ ] Step 2: GREEN——builtins.rs 加终端表（spec §3.2 全表：C/V/T/W/N、字母族 cmd→ctrl+shift、数字族、Tab 导航）。
- [ ] Step 3: RED——`passthrough_ctrl_space` 扩充 Terminal 分支（已在 Task 2 的测试函数里 loop ctx 补 Terminal case）→ 若失败即修 builtins。
- [ ] Step 4: `cargo test` 全绿；commit `feat(map): terminal context family + gating`。

### Task 6: sticky bind + Cmd+Tab 循环 + held-combo teardown

**Files:** Modify `src/engine/mod.rs`（DeviceState 粘滞块 + apply 的 bind 分支）。

**Interfaces:**
- Consumes: `Rule.sticky`。
- Produces: `apply` 新增粘滞行为；`DeviceState.bound: Vec<(in_code,out_code)>`；`pub fn release_held(state: &mut DeviceState) -> Vec<Out>`（仅释放 held combo 相关输出键/mods，保留 bound 的 sticky 项；供 Task 7 的上下文切换调用）。

**行为规格**（照 transform.py `auto_sticky` 抠）：规则 `sticky=true` 且匹配 → 输出 combo 的输出修饰键在输入修饰键（产生该 combo 的那个输入 mod）释放前**不得释放**；`bound` 中记录 (输入mod.code, 输出mod.code)；输入释放 → 输出对应键释放。非 sticky 保持 Task 2 行为。
- [ ] Step 1: RED——`tab_cycle`：windows 下输入 `(125,1)(15,1)(15,0)(15,1)(15,0)(125,0)`，CMD_TAB rule sticky=true → 输出流必须含 `ALT(56,1) ... TAB(15,1)(15,0) TAB(15,1)(15,0) ...` 且 **两次 tap 之间不得出现 ALT(56,0)**；最后输入 LMETA 释放 → ALT(56,0)。
- [ ] Step 2: GREEN——实现 sticky + bound 追踪。
- [ ] Step 3: RED——`teardown_releases`：非 sticky held combo 触发键释放后，未绑定输入仍按住的输出 mod 必须被释放；`shutdown` 断言全部 pressed 有 0 值。
- [ ] Step 4: GREEN。
- [ ] Step 5: `cargo test` 全绿；commit `feat(engine): sticky bind, cmd+tab cycle, teardown`。

### Task 7: FocusSource trait + KWin 适配器（dbus + 内嵌脚本）

**Files:** Create `src/adapters/mod.rs`(trait+注册表), `src/adapters/kwin.rs`, `src/adapters/kwin_script/main.js`(内嵌字符串 include_str!)，`src/adapters/none.rs`, Modify `src/main.rs`/`daemon`（上下文汇流）。

**Interfaces:**
- Consumes: zbus；`active_ctx`；Task 6 的 `release_held`（上下文切换时先释放 held combos，再来更新 context 并打日志）。
- Produces（契约见 spec §4 代码块，**不得内联**）:
  - `pub trait FocusSource: Send + Sync`：`id/install/uninstall/current/spawn_watch`
  - `pub fn best_for(config_adapter: &str) -> anyhow::Result<Box<dyn FocusSource>>`：
    auto → env `XDG_CURRENT_DESKTOP`/`KWIN_` 探测；uknown → err；v1 实现 kwin/none，gnome+windows = Unsupported 空桩
  - `pub struct KwinAdapter`：zbus 名 `org.mackeys.Daemon()`（独占=单实例），
    信号 `NotifyActiveWindow(String caption, String resourceClass, String resourceName)`
  - KWin js 脚本（抄 toshy kwin-script 精简）：`workspace.windowActivated` → `callDBus(...)`；
    install：写 `~/.local/share/kwin/scripts/mackeys-focus/` + dbus `org.kde.KWin /Scripting loadScript` + `start` + `reconfigure`；装载后主动触发一次重报
  - daemon 每次事件前读 `FocusTracker{ current }` 作 Ctx
- [ ] Step 1: 单测 terminal_classes 匹配（resourceClass/resourceName 大小写不敏感）→ `pure fn is_terminal(report, classes) -> bool`。
- [ ] Step 2: 实现 kwin.rs（装载/卸载/桥接），daemon 接线。
- [ ] Step 3: 手工联调清单输出（owner 验收：konsole 焦点 `journalctl --user -u mackeys` 显示 context=terminal）。
- [ ] Step 4: commit `feat(adapters): kwin focus bridge`。

### Task 8: setup / doctor / uninstall / devices / start-stop-status

**Files:** Create `src/setup/mod.rs`, `src/setup/doctor.rs`, `src/setup/service.rs`; Modify `src/main.rs`。

**Interfaces:**
- Produces: 
  - `setup [--takeover] [--no-fcitx5] [--dry-run]`：生成 `~/.config/mackeys/config.toml`（含注释全键位模板，**若已存在则不覆盖**）、systemd user unit（`After=graphical-session.target`、`Restart=on-failure`）、kwin 脚本装载、打印 doctor；`--takeover` = systemctl --user disable --now toshy-config（及其他 toshy-* 单元，失败容忍打印）；一切写入**幂等**（已存在跳过并报 already）
  - `doctor`：uinput 节点/权限、input 组、单元状态、**适配器 install 状态（按 best_for 结果）**、fcitx5 TriggerKeys[0]==Control+space、KRunner Alt+Space/KDE 同窗切换 Alt+`真实绑定（读 `$XDG_CONFIG_HOME/kglobalshortcutsrc` 或 defaults fallthrough 到 `/usr/share/kglobalaccel`）、其它 remapper 进程检测（keyd/xremap/xwaykeyz）、caps↔ctrl 交换方向校验；各项 PASS/FAIL + 修复建议；`--fix` 仅用户级
  - `uninstall`：停/禁 unit、kwin unloadScript、提示余项
  - `devices`：设备名/phys/kbtype 判定/是否纳管
  - `status`：unit status 包装
- [ ] Step 1: RED——`setup_idempotent`：临时 `$HOME` 隔离（xdg mock：XDG_CONFIG_HOME 指临时目录）连跑两次 → 第二次所有 “already”，文件数不变。
- [ ] Step 2: GREEN——实现幂等骨架。
- [ ] Step 3: doctor 逐项（每项一个纯函数单位测试，仅读参数）。
- [ ] Step 4: `cargo test` 全绿；commit `feat(setup): idempotent setup, doctor, uninstall`。

### Task 9: README + release + 真机验收 + tag

**Files:** Create `README.md`（正式），`.gitignore`(cargo)。

- [ ] Step 1: README（中文）：是什么/键位表/安装（`cargo install --path .` 或 releases 二进制）/config 例子/验证清单/卸载。
- [ ] Step 2: `cargo build --release`；真机 `setup --takeover` + doctor 全绿（Toshy 被停）。
- [ ] Step 3: 真机验收按 spec §6 七条勾选（owner 手测，agent 只依据 doctor 输出与日志）。
- [ ] Step 4: commit `docs: readme, usage` + `git tag v0.1.0` + push。

## Self-Review 后记

- 规格覆盖检查：§3.1 桌面=GUI 在 Task 5 `active_ctx`；§3.3 XWayland 无分支=天然；§7 风险在 Task 3/8；§6.6 热插拔——**计划缺失**，已并入 Task 3 (Step 2 的设备管理循环需支持 udev monitor add/remove)——Task 3 Interfaces 补一行:`udev monitor 线程 add/remove → 重建 reader`。
- 步依序执行+每步可验证；类型一致；Review Focus 五项均有宿主测试。
