# mackeys

**三个键位习惯，一个二进制。** 在 Linux 上重写 Toshy 验证过的核心机制：
`evdev` 独占抓取全部键盘 → 上下文引擎（GUI / 终端分流）→ `uinput` 虚拟键盘吐出。

- 无托盘、无 GUI、无后台服务数据库——配置就是一个 TOML 文件
- GNOME/GNOME 之外全按 Plasma 6 Wayland 验证（GNOME/Hyprland 适配器接口留桩待实现）

```text
当前运行时依赖：Linux ≥ 6.x / 任一 Wayland 会话（推荐 KDE Plasma 6）
编译依赖：Rust stable（rustup / 发行版包皆可）
```

## Quickstart —— 三分钟上手

```bash
# 0. 前置：确认你在 input 组（macOS 无需 sudo 装驱动；权限问题 doctor 会提示）
groups | grep input   # 若无：sudo usermod -aG input $USER 后重新登录

# 1. 克隆并构建
git clone https://github.com/lovelock/mackeys && cd mackeys
cargo build --release

# 2. 一键安装（写配置模板 + systemd 用户单元 + KWin 焦点桥接脚本；
#    --takeover 会顺带 disable 掉 Toshy 等同类 remapper，避免抢抓 evdev）
./target/release/mackeys setup --takeover

# 3. 启用并自检
systemctl --user enable --now mackeys
./target/release/mackeys doctor        # 期望 7 项全 [PASS]
```

## 立刻验证（30 秒即可试完）

| 动作 | 期待 |
|------|------|
| GUI 应用里按 Cmd+C、Cmd+V | 复制/粘贴（= 应用收到 Ctrl+C/V） |
| Cmd+Tab | 应用切换器（按住 Cmd 循环，松手确认，macOS 式） |
| Cmd+Space | KRunner 应用启动器 |
| 终端里 Cmd+C / Cmd+V | 终端复制/粘贴（= Ctrl+Shift+C/V，不误发 SIGINT） |
| 终端里按 **Caps** | **真 Ctrl**：SIGINT、tmux 前缀（Caps+B） |
| 物理左 Ctrl | 大写锁定切换 |
| 任意窗口 Caps+Space | 切换中文输入法（fcitx5/ibus），永不被拦截 |

> 记忆法：**物理 Caps 就是你的 Ctrl 键**（HHKB 手感）；macOS 的 Cmd 就是键盘上“空格旁”的那个 Alt。
> 所有键位一张大表在下方[键位表](#键位表默认即作者偏好)。

## 键位表（默认即作者偏好）

| 物理键 | 效果 |
|--------|------|
| Caps | 真 Left Ctrl（所有应用包括终端） |
| 物理左 Ctrl | CapsLock |
| Alt（空格旁） | Cmd：GUI 里按下去整个字母族全是 Ctrl 族 |
| Cmd+Tab / Cmd+Shift+Tab | Alt+Tab / Alt+Shift+Tab（sticky 循环，松手确认） |
| Cmd+` | Alt+`（同应用窗口切换） |
| Cmd+Space | Alt+Space → KRunner |
| Cmd+Q | Alt+F4 |
| Cmd+, | Ctrl+, |
| Cmd+←/→、Cmd+↑/↓ | Home/End、Ctrl+Home/End |
| Win(=Option)+Backspace/Delete | Ctrl+Backspace / Ctrl+Delete（删词） |
| Win+←/→ | Ctrl+←/→ |
| Cmd+H | 吞键（macOS 无等价的“隐藏应用”不落到应用） |

**终端分流的特殊处理**（窗口类自动识别：konsole/ghostty/kitty/alacritty/wezterm/foot…）：

| 组合 | 效果 |
|------|------|
| Cmd+C / V / X / … | Ctrl+Shift+C / V / X / …（终端复制粘贴） |
| Cmd+0..9 | Ctrl+0..9 |
| Cmd+←/→ | Ctrl+PageUp/PageDown |
| Caps（永远一样） | 真 Ctrl：SIGINT、tmux、`Ctrl+L` 清屏全按 Linux 原生语义 |

**守则**：`Ctrl+Space` 在任何上下文永远不会被拦截——它是你的中文输入法触发键。

## 配置 `~/.config/mackeys/config.toml`

默认值即作者偏好，TOML 是差分（没提到的项用默认）。完整可注释模板由 `setup` 生成。

```toml
[general]
kbtype_default = "windows"         # windows | apple
adapter = "auto"                   # auto | kwin | gnome | none | windows
# swap_caps_lctrl = true
# disable_builtins = ["cmd_tab"]   # 按内置 id 关闭（id 清单见 src/map/builtins.rs）
# exclude_devices = ["Gaming Keyboard"]   # 按名匹配排除（正则/子串/前缀均可）

[targets]
# terminal_classes = ["^konsole", "ghostty", "alacritty", "kitty", "wezterm", "foot"]

[launcher]
# cmd_space = "alt+space"          # 或 "meta+space"；"none" = 吞键

[[rules]]                          # 你的个人规则（first-match-wins，排最前）
input = "cmd+w"
output = "ignore"                  # 或 "ctrl+..." / "alt+f4"
context = "gui"                    # gui | term | any
```

## 日常命令

| 命令 | 说明 |
|------|------|
| `mackeys status` / `start` / `stop` | systemd 单元控制 |
| `mackeys doctor` | 逐项环境自检（PASS/FAIL + 修复建议）|
| `mackeys devices` | 当前设备清单 + 是否被接管 |
| `mackeys dump` | 打印编译后的规则表（first-match 顺序） |
| `mackeys setup` | 幂等重跑（升级 / 改配置后用来重装工件） |
| `mackeys uninstall` | 反向卸载（顺序：停单元→卸 kwin 脚本→删 unit） |

## 疑难排查

常见失败场景与对策（都是 doctor 会直接提示的）：

| doctor 提示 | 原因 | 处理 |
|-------------|------|------|
| uinput mode ... needs udev rule | `/dev/uinput` 权限不足 | `sudo usermod -aG input $USER` 重新登录，或加 udev 规则 |
| kwin adapter: script NOT loaded | KWin 已被手动 unload / 资源锁没拿到 | 重跑 `mackeys setup`；或 `systemctl --user restart mackeys` |
| no conflicting remappers FAIL | toshy/keyd/xremap 还在运行 | `systemctl --user stop --now <unit>` 后重试 |
| 键位行为和 README 不符而 doctor 全绿 | 用户规则/内置规则冲突 | `mackeys dump` 对照 first-match 顺序 |

## 卸载

```bash
mackeys uninstall
```

会停掉单元、卸 KWin 脚本、删 unit 文件；配置留在 `~/.config/mackeys/`（如需彻底清除手动删除该目录）。

## 架构

```
src/
  types     — Act / Out / Combo / Rule / RuleSet（全部 pub 接口的基石）
  engine/   — 纯函数键位引擎（Act → Vec<Out>；无设备依赖，完全可单测）
  daemon/   — evdev 读者线程 + engine + uinput sink；每设备独立 Keystate；kbd 热插拔
  map/      — token DSL（cmd/opt 角色别名解析）+ 初始化内置表（GUI + Terminal 两语义）
  adapters/ — 可替换组件：【FocusSource trait】 + 注册表；v1 kwin/none；gnome/windows 接口预桩
  setup/    — setup / doctor / uninstall / service 生成
```

**License**：MIT。规格/决策日志归档在 `docs/superpowers/`（specs + plans）。
