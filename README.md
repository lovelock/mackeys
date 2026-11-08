# mackeys

macOS 式键位语义的 Linux 实现。单二进制 Rust 守护进程：`evdev` 独占抓取全部键盘 → 上下文引擎（GUI/终端分流）→ `uinput` 虚拟键盘吐出。干净从零重写，核心机制借鉴 **Toshy** 验证过的设计（held-combo/compositor-driven repeat、sticky bind、KWin 脚本报焦点），无托盘、无 GUI、无 DB 后台。

> 免责：Windows 键盘布局为默认（也支持 Apple 键盘）。开发机为 Plasma 6 Wayland，GNOME/Hyprland 适配器留了接口尚未实现。

## 键位表（默认即作者 HHKB/macOS 工作流）

| 物理键 | 效果 |
|--------|------|
| **Caps** | 真 Ctrl（终端 SIGINT、tmux 前缀、Ctrl+Space 切输入法）|
| 物理左 Ctrl | CapsLock |
| **Alt（空格旁）= Cmd** | Cmd+C 复制、Cmd+V 粘贴、Cmd+T 新标签、Cmd+W 关标签…GUI 里 = Ctrl 族 |
| Cmd+Tab / Cmd+Shift+Tab | Alt+Tab / Alt+Shift+Tab（macOS 循环式，松手确认）|
| Cmd+` | Alt+` 同应用窗口切换 |
| Cmd+Space | Alt+Space → KRunner |
| Cmd+Q | Alt+F4 关窗口 |
| Cmd+, | Ctrl+, |
| Cmd+←/→ / ↑/↓ | Home/End（文档=Ctrl+Home/End） |
| Win(=Option)+Backspace/Delete | Ctrl+Backspace/Delete 删词 |
| Win+←/→ | Ctrl+←/→ |
| Cmd+H | 吞键（macOS 无 Linux 等价，不落到应用）|

**终端中**自动分流：Cmd+C/V/X → Ctrl+Shift+C/V/X（复制粘贴），Cmd+其余字母 → Ctrl+Shift+X，Cmd+0..9 → Ctrl+0..9，Cmd+←/→ → Ctrl+PageUp/Down。终端里的物理 Caps 永远是真 Ctrl（SIGINT/tmux 语义不被篡改）。

**原则**：Ctrl+Space 任何模式下永不拦截（fcitx5 触发键）。

## 安装（源码）

```bash
git clone https://github.com/lovelock/mackeys
cd mackeys
cargo build --release
./target/release/mackeys setup --takeover   # 写配置/单元/适配器 + 停用 toshy 等冲突者
systemctl --user enable --now mackeys
./target/release/mackeys doctor             # 环境自检，全部 PASS
```

## 常用命令

| 命令 | 作用 |
|------|------|
| `mackeys setup [--takeover]` | 幂等安装：config 模板 + systemd 单元 + KWin 脚本；`--takeover` 停用 toshy |
| `mackeys doctor` | 逐项环境自检（PASS/FAIL + 修复建议） |
| `mackeys status` / `start` / `stop` | systemd 单元控制 |
| `mackeys devices` | 设备清单 + 纳管状态 |
| `mackeys dump` | 打印编译后的规则集（first-match 顺序）|
| `mackeys uninstall` | 反向卸载 |

## 配置 `~/.config/mackeys/config.toml`

默认值即作者偏好，TOML 只做差分（完整模板见 `setup` 生成）：

```toml
[general]
kbtype_default = "windows"     # windows | apple
adapter = "auto"               # auto | kwin | gnome | none | windows
# swap_caps_lctrl = true
# disable_builtins = ["cmd_tab"]
# exclude_devices = ["Gaming Keyboard"]

[targets]
# terminal_classes = ["^konsole", "ghostty", "alacritty", "kitty", "wezterm", "foot"]

[launcher]
# cmd_space = "alt+space"      # 或 "meta+space" / "none"（吞键）

[[rules]]                      # 用户规则（first-match-wins，排内置之前）
input = "cmd+w"
output = "ignore"
context = "gui"                # gui | term | any
```

## 验证清单（装完自测）

1. GUI：Cmd+C 复制、Cmd+V 粘贴、Cmd+Q 关窗、Cmd+Tab 循环切应用（按住 Cmd 循环，松手确认）
2. Cmd+Space → KRunner
3. 终端：Cmd+C/V = 复制/粘贴；物理 Caps = 真 Ctrl（SIGINT / tmux 前缀用 Caps+B）
4. 物理左 Ctrl = 大写切换
5. Caps+Space = 切输入法（fcitx5），任何场景不被拦截

## 卸载

```bash
mackeys uninstall
```

## 架构

- `engine/`：纯函数键位引擎（`Act → Vec<Out>`），无设备依赖，全可单测
- `daemon/`：evdev 读者线程 + engine + uinput sink；每设备独立 Keystate；grab/退出全状态同步
- `map/`：token DSL（物理 evdev 码 + 角色别名）→ 内置表编译；用户规则排最前
- `adapters/`：**FocusSource trait + 注册表**（DE 无关核心只见 trait）：v1 = kwin；none = 回退；gnome/windows 接口预留
- `setup/`：幂等 setup、doctor（纯函数检查项）、uninstall、service 生成

构建 licensee：MIT。所有键位行为说明书见 `docs/superpowers/specs/`。
