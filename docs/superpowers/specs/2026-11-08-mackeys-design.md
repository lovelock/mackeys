# mackeys 设计文档（Toshy 精华内核的干净 Rust 重实现）

date: 2026-11-08 · owner: frost · status: draft for approval

## 1. 意图与由来

用户在 Wayland（KDE/Plasma，将来可能其他合成器）上需要 macOS 习惯的键位语义。
经历：Toshy（功能全但盘太大、上下文机器不可控、问题排查成本高）→ keyd / xremap（不典型、语义表达不够）。
本项目把 Toshy 已验证的**内核翻译机制**用 Rust 干净重写，配一个最小 TOML。

一句话：**`evdev 独占抓取 → 上下文引擎 → uinput 虚拟键盘吐出`** 的常驻守护进程，
外加一条 `setup` 命令完成环境自举。

## 2. 非目标（不做清单）

- 无托盘、无 GUI、无图形设置（永远不修）——TOML 文件即全部配置面
- 不做：机器指纹、升级切片机制、多语言键盘、远程/Synergy 增强、假键盘 Cbk 处理、Toshy 的几十种 DE 分支
- 不改 XKB/内核层（不做 XKB options Scanner），适配只进用户空间
- 不做 per-app 用户规则 UI；用户靠 TOML 的 `[[rules]]` 表追加

## 3. 按键语义（核心规格——按用户 2026-11-08 更正版）

默认布局 = Windows 键盘（BY Tech 等）；kbtype=Apple 键盘时把 Cmd/Opt 的物理换位自动调整。

| 物理 | 输出 | 备注 |
|------|------|------|
| Caps | LEFT_CTRL | 字面真 Ctrl，终端里即 SIGINT/tmux 语义 |
| 左 Ctrl | CAPSLOCK | 完整对调 |
| 左 Alt（空格旁） | RIGHT_CTRL | = Cmd：Cmd+C → 应用收 Ctrl+C（复制）|
| 左 Win | LEFT_ALT | 标准 Alt 语义（菜单、Alt+Tab 组合等），兼作 Option 角色 |
| Ctrl+Space | **直通** | 永不拦截 —— fcitx5 切换键（Toshy SPOTL 教训） |
| Caps+Space | 同上 | 即 Ctrl+Space 直通 |

### 3.1 GUI 上下文键位表（默认启用）

| 组合 | 输出 | 语义 |
|------|------|------|
| Cmd+C/X/V | Ctrl+C/X/V | 复制/剪切/粘贴 |
| Cmd+Z / Cmd+Shift+Z | Ctrl+Z / Ctrl+Shift+Z | undo/redo |
| Cmd+A/S/F/T/W/N/L/...（字母族） | Ctrl+同键 | 保存/查找/新标签/关标签 |
| Cmd+Tab / Cmd+Shift+Tab | Alt+Tab / Alt+Shift+Tab | 应用切换 |
| Cmd+` | Alt+` | 同应用窗口切换（KDE 行为同 ring 的一种） |
| Cmd+Space | Alt+Space | KRunner（应用启动器） |
| Cmd+←/→、Cmd+↑/↓ | Home/End、Ctrl+Home/End(文档) | 行首/尾、文首/尾 |
| Cmd+W | Ctrl+W | 关标签 |
| Cmd+H | 忽略（吞键） | macOS 语义无 Linux 等价（Toshy 默认亦吞） |
| Cmd+Q | Alt+F4（关闭窗口） | KDE 默认 |
| Cmd+, | Ctrl+, | 常见首选项键 |
| Option(左 Win)+Backspace/Delete | Ctrl+Backspace/Delete | wordwise 删词 |
| Option(左 Win)+←/→ | Ctrl+←/→ | 词内跳词（终端外） |

（明确：物理键盘上本工具只有 Cmd 位 = 左 Alt 一处；Windows 键盘的双 Alt、IBM/Chromebook 布局留给 kbtype 分支，首版以用户实际键盘 Windows 布局 + Apple 自动识别为限。）

### 3.2 终端上下文键位表（KGenTerm 语法类）

| 组合 | 输出 | 备注 |
|------|------|------|
| Cmd+C | Ctrl+Shift+C | 复制（无选区时不误发 SIGINT——因为输出带 Shift，XKB 传 Ctrl+Shift+C 的终端复制键） |
| Cmd+V | Ctrl+Shift+V | 粘贴 |
| Cmd+T / Cmd+W | Ctrl+Shift+T/W | 标签 |
| Cmd+N | Ctrl+Shift+N | 新窗口 |
| 其余 Cmd+字母 | Ctrl+Shift+同键 | 与 Toshy "General Terminals" 一致 |
| Cmd+←/→ / Cmd+数字 | Ctrl+PageUp/PageDown、Ctrl+0… | 标签导航 |
| Ctrl+C（由 Caps 发出） | 原样 | SIGINT/tmux 前缀，永不被篡改 |

判定：窗口类 ∈ `terminal_classes`（默认含 konsole, ghostty, alacritty, kitty, wezterm,
org.wezfurlong.wezterm, foot, rio 等，config 可增删）→ 终端上下文。

### 3.3 上下文引擎（Toshy 精华照搬）

- 每设备独立 `Keystate`：inkey、output-side key、modifier 合并、多用途键（tap/hold）——`multipurpose_modmap`
  规避：`Caps/左Ctrl 完整对调`是**纯 modmap**（不动 keystate 的 key 标签）模式，避免 Toshy
  "在 signals mod- map 之前 rewrite keystate 导致 multi 失效" 的坑
- 组合输出时输出键保持按下（**compositor 驱动 repeat**），触发键抬起即 `teardown`
  ——照抄 transform.py 的 `HeldComboContext` 设计，不实现自己的 autorepeat
- 回滚预防：SPOTL 式全局拦截**不存在**；Ctrl+Space（含 Caps+Space）直通 → fcitx5 可用
- 焦点上下文：KWin dbus 脚本（精简版 toshy-dbus-notifyactivewindow）：KWin 前台窗口类变化 → dbus 信号 → daemon 更新
  `ctx_app_is_terminal`；其他合成器降级：无窗口类 = GUI 上下文（config 可强制 terminal/GUI）
- XWayland 不做特殊分支：evdev 层拦截意味着 X11/Wayland 应用一视同仁（比 xwaykeyz 只改 X 层更彻底）

## 4. 架构

```
crates.io 依赖（全部成熟）：tokio(异步事件循环)、evdev(输入枚举+grab)、
libc/nix(uinput 裸 ioctl 或 uinput 包)、zbus(dbus)、serde+toml(配置)、tracing(日志)、anyhow
```
```
src/
  main.rs        — 子命令分发: run(默认) / setup / doctor / start|stop|status / devices / dump / uninstall
  daemon/        — 事件循环: input threads → engine → output；FOCUS 消费
  engine/        — Keystate、modmap、keymap 匹配(first-match-wins)、held-combo/repeat、
                   bind/sticky 指令、multitap(暂缓)
  map/           — TOML → 规则编译（内嵌 default_preset == 上面的键位表）
  adapters/      — 【可替换组件】`FocusSource` trait + 注册表 best_for(env)；
                   v1: kwin.rs（嵌 kwin 脚本+dbus 对接）+ none.rs（无报告回退）；
                   预留 gnome.rs / windows.rs 空实现（Unsupported）；
                   general.adapter = "auto"|"kwin"|"gnome"|"none"|"windows"
  setup/         — doctor 环境自检: uinput 权限(uaccess)、evdev 组、service、
                   适配器 install 状态、fcitx5 trigger 是 Control+space(提示而非改写)、
                   Caps/LCtl 交换验证提示
  service/       — systemd user unit 生成
```

**FocusSource trait（DE 可替换组件的契约，builder 不得内联到 daemon）：**

```rust
pub struct WindowReport { pub caption: String, pub resource_class: String, pub resource_name: String }
pub trait FocusSource: Send + Sync {
    fn id(&self) -> &'static str;                       // "kwin"|"gnome"|"none"|...
    fn install(&self) -> anyhow::Result<()>;            // 外部工件：kwin 脚本文件+装载；gnome: 扩展装载
    fn uninstall(&self) -> anyhow::Result<()>;
    fn current(&self) -> Option<WindowReport>;          // 即时快照
    fn spawn_watch(self: Arc<Self>, tx: mpsc::Sender<Option<WindowReport>>) -> anyhow::Result<()>;
}
pub fn best_for(config_adapter: &str) -> Box<dyn FocusSource>;   // auto→按 env 探测；未知→err
```

- `run`：默认加载 `~/.config/mackeys/config.toml`；缺失时**不悄悄生成**——`setup` 生成覆盖，
  `run` 报错提示先 setup（显式而非魔法）
- `setup`：doctor 前置 → 写 service → 启动提示 → 打印验证清单（即"额外多一步"：
  像 macOS 的修饰键设置确认面板那样逐项 PASS/FAIL）
- `doctor`：全部自检可重复运行；`--fix` 尽力修复权限类项
- `devices`：列出设备名/phys/kbtype 判定（配置调试用）
- `start/stop/status`：systemctl --user 包装

## 5. 配置文件（唯一配置面）

`~/.config/mackeys/config.toml`（全部键可省略，默认 = 上表 + 用户偏好）：

```toml
# mackeys 配置 — 只有覆盖默认的键才需要写
[general]
kbtype_default = "windows"            # windows | apple | ibm | chromebook
adapter = "auto"                      # auto | kwin | gnome | none | windows
swap_caps_lctrl = true                # Caps↔左Ctrl 完整对调
[targets]                              # 终端窗口类（正则）
terminal_classes = ["^konsole", "ghostty", "alacritty", "kitty", "wezterm", "foot", ...]
[launcher]                             # Cmd+Space 去哪儿
cmd_space = ["alt", "space"]          # 输出组合；置 "none" 即关闭
[per_device]                           # 按设备名覆盖 kbtype
# "BY Tech Gaming Keyboard" = "windows"
[[rules]]                              # 用户追加/覆盖规则（first-match-wins、插入在最前）
input = ["alt", "space"] context = "gui" output = ["alt", "f2"]
```

设计原则：默认即用户偏好 baked-in（这题从本会话整定），TOML 只做差分。

## 6. 成功标准（按用户更正版）

1. **Cmd+C → 复制**（GUI 直接 Ctrl+C 到应用；终端 → Ctrl+Shift+C）
2. 终端里 **Caps+C = SIGINT**；tmux 前缀 Caps+B 每单
3. **Caps+Space（=Ctrl+Space）→ fcitx5 切换**，任何模式下永不拦截
4. **物理左 Ctrl = CapsLock**
5. Cmd+Tab、Cmd+`、Cmd+Space→KRunner、wordwise、Cmd+,：三上下文全过（GUI/终端/桌面）
6. 重启持久（systemd user units）；新插键盘自动生效（evdev 枚举热更新）
7. `mackeys doctor` 全绿；卸载 = 停用 systemd 单元 + 移除 udev 权限规则 + （若曾装 kwin 脚本）卸载 kwin 脚本，全部由 `mackeys uninstall` 完成

## 7. 风险与关键决策记录

- **uinput 权限**：`/dev/uinput` 默认 root-only → 方案：`/etc/udev/rules.d` + `uaccess`
  tag 或 systemd-logind group（setup 输出明确指令；Wheel 用户手动一次）
- **evdev grab 竞态**：合成器同时监听 → evdev crate 的 grab() Exclusive 验证过（Toshy/xkeysnail 同工艺）
- **多设备**：每设备独立 Keystate（Toshy 教训：共享 keystate 导致多键盘互相污染修饰键）
- **自动重复**：禁止自造 repeat，输出端 held 键交给合成器 repeat（Toshy HeldComboContext 语义）
- **kwin 脚本装载**：按 toshy_kwin_script_setup.py 的 dbus 官方通道
  (org.kde.KWin /Scripting loadScript + start) 精简实现
- **fcitx5**：不改写它的配置（setup 只检查并提示），避免预设互相污染
- **DE 适配器为可替换组件（2026-11-08 owner 拍板）**：`FocusSource` trait 契约 + 注册表
  （见 §4 代码块）；kwin 为 v1 内置实现，none 为无报告回退；gnome/windows 为空实现预留。
  daemon 核心只 import trait；kwin 特化细节（脚本、dbus 通道）永不出 adapters/kwin.rs。
  配置键 `general.adapter`（默认 auto）选择；doctor 报告所选适配器及其 install 状态。
- **与 Toshy/其他 remapper 互斥**：doctor 检测到 xwaykeyz/keyd 进程时警告 + 可 systemctl 停用提示

## 8. 已定决策（round-2，2026-11-08，构建者检查后全部并入）

用户拍板（5 项）：
- **Cmd+Tab = macOS 循环式**：按住 Cmd 循环、松手确认 → 引擎必须移植 Toshy 的
  `bind + auto_sticky`（输入 Cmd 按住期间输出 Alt 保持按下），这是引擎最高优先项；
  文档此前 §3.3 的"触发键抬起即 teardown"文字对该条目不充分，以本节与
  transform.py 的 sticky 语义为准
- **桌面上下文 = GUI 上下文**（无窗口焦点时按 GUI 表处理）
- **Cmd+Space 默认输出 = Alt+Space**（本机 KRunner 解析值，doctor 校验 kglobalaccel 真实绑定并警告）
- **接受物理 CapsLock LED 不亮**（状态由虚拟设备承担；doctor 说明）
- **开发/接管期 Toshy 停用**：`mackeys setup --takeover`（停 toshy-config 等 toshy 单元并 disable）；
  doctor 常驻检测其他 remapper（keyd/xremap/xwaykeyz 进程）并警告

并入的构建者默认（builder-check (b) 1–25 逐条落地）：
- 输入 `value=2`（内核 autorepeat）全局丢弃——重复只由合成器驱动；不移植 RepeatCache
- SYN_DROPPED → EVIOCGKEY 重同步 + 释放不再成立的输出键
- grab 起始从 EVIOCGKEY 同步按住态；SIGTERM/exit 时 release-all + 全设备 ungrab
- 只抓含字母键的 evdev 节点（Consumer/System Control 节点放行原生）；跳过自己的 uinput 设备；`[per_device] exclude_devices`
- 热插拔：udev monitor add/remove → 设备重建（含 suspend/resume 重枚举）
- 终端表**覆盖叠加** GUI 表；规则 token 命名 = 物理 evdev 键 + 角色别名
  （`cmd`/`opt`：Windows 布局解释为 lalt/lmeta，Apple 布局为 lmeta/lalt；`alt`/`meta` 泛指左）
- `[general] disable_builtins = [...]` 按内置规则 id 关闭；`[[rules]]` 用户提供规则
  first-match-wins 排最前；`output = "ignore"` 吞键（Cmd+H 用）
- `cmd_space = "none"` = **吞键**（直通会撞 fcitx5 的 Ctrl 语义）
- v1 kbtype 仅 windows/apple（USB 0x05AC 或名称识别 + per_device 覆盖）；ibm/chromebook 拒识并提示
- `dump` = 输出编译后的 modmap+keymap（first-match 顺序）、设备表、每设备当前上下文
- `uninstall` 子命令：停/禁单元、移除 udev 规则（若 setup 写入）、kwin `unloadScript`
- dbus：`org.mackeys.NotifyActiveWindow(caption, resourceClass, resourceName)`，
  daemon 独占 bus name 即单实例守卫
- systemd unit：`After=graphical-session.target`、`Restart=on-failure`
- 依赖修正：输出用 `evdev` crate 的 `VirtualDeviceBuilder`（无需单独 uinput 包）；
  输入端每设备阻塞读线程 → channel → 引擎单消费
- `doctor --fix` 只做用户级修复（udev /etc 层只打印命令；本机 `/dev/uinput` 已是
  `root:input 0660` 且用户在 input 组——doctor 按实际状态判定）
- doctor 校验 KRunner/Alt+` 的 KWin 实际绑定
- KWin 脚本装载后让 daemon 主动触发一次当前焦点重报（避免"启动即 GUI 上下文"卡住）
- 终端类匹配 = resourceClass 或 resourceName，大小写不敏感
- 默认假设 ANSI/US 物理布局，规则全部 evdev 键码 → 布局无关；不引入 Toshy 的 XKB correction
- TTY/VT（Ctrl+Alt+F1..F7）：EVIOCGRAB 不拦内核 VT 处理，物理端按原始语义可用；
  uinput 流内另有交换语义可达，不构成双触发——doctor 文档说明
- 上下文切换毫秒窗口内 Cmd+C 可能落到旧上下文（Toshy 同样如此）：切换时 teardown held combos + 日志记录

## 9. 未定（收到否决即可动）

- 名字 mackeys（可改名）
- multitap / sticky 单键 / forced-numpad / Enter2Cmd：首版不做（backlog）
- gnome/wlroots/hyprland 适配器：接口预留，首版空实现
- "多一步"= `doctor` 自检面板已按 (a) 理解实现；若要 (b)（深入 KDE/DE 原生设置区写等价项）请否决
