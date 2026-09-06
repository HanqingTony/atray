# atray Linux Wayland 状态说明（交接文档）

> 2026-09-06 · 101（Debian 13 trixie + KDE Plasma 6.3, Wayland 会话, NVIDIA）· 对应版本 v1.2.0
> 状态：**✅ 已全通**（2026-09-06 深夜）。此前卡点「按键不触发」根因=setShortcut 缺
> SetPresent 旗标，已修复并物理按键验证（详见下文「已解决卡点」与 kglobalaccel.rs 坑 10）。

## 目标

Linux Wayland（当前 KDE）上一个原生程序：**全屏窗口承载任意 web 应用（anm-web
等）+ 全局热键呼出/隐藏**。不追求 Windows 式透明覆盖层（anm-web 自带深色背景）。

## 当前可工作项（101 实测）

| 项 | 状态 | 说明 |
|---|---|---|
| 全屏 | ✅ | 强制 `GDK_BACKEND=x11`（XWayland）后 1920x1080 全屏正常（tao 的 Wayland fullscreen 有几何 bug，见下） |
| 窗口/托盘/renderer | ✅ | WebKitGTK X11 下正常 |
| 热键注册（KGA 直连） | ✅ | 固定组件 `atray`，`toggle` 动作，键 Alt+Shift+A(167772225) 写入 `~/.config/kglobalshortcutsrc [atray]`，重启保留 |
| 幂等 | ✅ | 组件恒 1 个（不再堆积）；用户改键后重启保留（宽松同步不覆盖） |
| portal 通道 | ⚠ | 代码完整（跨 GNOME/Hyprland），但 Debian 13 的 portal-kde 有打包缺陷（见坑 3）；KDE 上已改走 KGA |
| **按键触发** | ✅ | **根因：setShortcut flags 缺 SetPresent(0x2)**——只传 NoAutoloading(0x4) 键能存但动作不 present，`setActive()` 抓键前置条件不满足 → 键不进 `_active_keys` → 按键无匹配无信号。修复：flags=0x6（FLAG_SET_KEY）。物理按键 Alt+Shift+A 呼出/隐藏实测通过 |

## 架构（lib.rs 热键后端分层）

```
Linux Wayland:
  KDE (XDG_CURRENT_DESKTOP~KDE) → kglobalaccel.rs（KGA 直连, 固定组件, 幂等）
  其它 Wayland (GNOME/Hyprland…) → portal.rs（XDG GlobalShortcuts portal）
X11 / Windows → native（tauri-plugin-global-shortcut）
```

选择逻辑在 `lib.rs` setup（`Backend::{Native,Kga,Portal}`）。前端 get_config
返回 `backend` 字段（"native"|"kga"|"portal"）；前端只在 `=== 'portal'` 时走
"系统对话框确认"交互，kga/native 都是抓组合即时保存。

## 已解决卡点（2026-09-06）：按键不触发

**根因**：`setShortcut` 的 flags 只传了 `NoAutoloading(0x4)`，漏了 `SetPresent(0x2)`。
kglobalacceld 里 `GlobalShortcut::setActive()` 抓键前置条件是 `_isPresent`
（kglobalacceld.h：`SetPresent=2, NoAutoloading=4, IsDefault=8`）：
不 present → 键永远不进 registry 的 `_active_keys` → `processKey()` 匹配不到 →
不发 `globalShortcutPressed`。真实 KDE 框架客户端都带 SetPresent，纯 D-Bus 第三方
最容易漏。

**定位过程**（接手者可复现）：
1. `busctl --user call org.kde.kglobalaccel /component/atray org.kde.kglobalaccel.Component isActive` → **false**（kwin/plasmashell 是 true）——关键诊断信号
2. kglobalacceld 源码（invent.kde.org/plasma/kglobalacceld）：`globalshortcut.cpp`
   `setActive()`/`setIsPresent()` + `kglobalacceld.cpp` `setShortcutKeys()` 的
   `setPresent = flags & SetPresent` 逻辑
3. 诊断捷径：`Component.invokeShortcut` 可绕过抓键直接发信号——能测 atray
   监听/分发链路，但与真实按键无关，别被它误导

**修复**：设键 flags = `SetPresent | NoAutoloading = 0x6`（kglobalaccel.rs
`FLAG_SET_KEY`；因 present 无读接口，对账时对期望动作无条件 setShortcut）。
验证：isActive=true；dbus-monitor 见 `globalShortcutPressed('atray','toggle',…)`；
atray 收到并切换窗口；**物理按键 Alt+Shift+A 呼出/隐藏通过**。

**附带设计升级（v2 对账）**：config.json=唯一期望态，单一 `reconcile_all()` 在
启动/IPC 变更/watchdog（20s stat kglobalshortcutsrc，外部改键即还原）三处触发；
系统侧偏离一律收敛回配置（产品决策：不在系统设置改 atray 动作）。

## 踩坑全集（写代码前必读）

详见 `src-tauri/src/kglobalaccel.rs` 文件头（完整 10 条 + 验证命令），摘要：

1. **Tauri global-shortcut 插件 Wayland 不可用**（X11 XGrabKey only）
2. **portal 通道不幂等**：每次 session 新建 token_ashpd_* 组件 → 堆积 → 同键
   冲突 → 热键全失效；portal 无固定组件概念，只能事后 cleanup
3. **Debian 13 portal-kde 6.3.5 无绑定对话框**：上游 tarball 缺 GlobalShortcutsDialog
   等 QML（二进制 qrc 只有 UserInfoDialog.qml）；BindShortcuts 静默挂起。
   修复：`scripts/fix-kde-portal-qml.sh`（补 ki18n qmldir + master QML 模块）
4. **tao/WebKitGTK Wayland fullscreen 几何 bug**：状态 true 但窗口不全屏；
   原生 GTK 正常 → tao 层问题 → 强制 GDK_BACKEND=x11（lib.rs run() 注释）
5. **actionId = 4 元素** [组件, 动作, 组件友好名, 动作友好名]；doRegister 才创建
   组件（getComponent 不创建）
6. **setShortcut 需 flags=NoAutoloading(0x4)**：否则 daemon 走 autoload 分支
   忽略新键（返回 [0]）；busctl 调它报参数错误，用 gdbus/zbus
7. **键编码 = Qt QKeySequence int**（mods|keycode）：Alt+Shift+A=167772225
8. **allShortcutInfos 签名 a(ssssssaiai)=6串+ai+ai**（不是 aai），反序列化错
   则 cleanup 静默失效 → 组件堆积
9. **zbus5 API 与 zbus4 不同**：receive_signal 泛型语义、add_match_rule 在
   fdo::DBusProxy、MatchRule::builder 部分方法返回 Result、MessageStream
   需规则才收广播
10. **setShortcut 必须带 SetPresent(0x2)**（⚠ 按键不触发根因，见上节）：只带 0x4
    键能存但动作不 present → 不抓键。设键一律 0x6 = SetPresent|NoAutoloading；
    `Component.isActive()`=false 是漏旗标的可靠诊断信号

## 验证过的命令（可直接用）

```bash
# 组件/动作/键查询
busctl --user call org.kde.kglobalaccel /kglobalaccel org.kde.KGlobalAccel allComponents
busctl --user call org.kde.kglobalaccel /component/atray org.kde.kglobalaccel.Component allShortcutInfos

# 设键（flags=6 = SetPresent|NoAutoloading，缺 SetPresent 不抓键！）
gdbus call --session --dest org.kde.kglobalaccel --object-path /kglobalaccel \
  --method org.kde.KGlobalAccel.setShortcut \
  "[\"atray\",\"toggle\",\"atray\",\"呼出/隐藏 atray 覆盖层\"]" "[167772225]" "6"

# 注册动作（幂等）
busctl --user call org.kde.kglobalaccel /kglobalaccel org.kde.KGlobalAccel \
  doRegister as 4 atray toggle atray "呼出/隐藏 atray 覆盖层"

# 清理动作
busctl --user call org.kde.kglobalaccel /kglobalaccel org.kde.KGlobalAccel \
  unregister ss atray toggle

# 信号监视（排查卡点用）
dbus-monitor "interface='org.kde.kglobalaccel.Component'"
```

## 部署

```bash
bash deploy-linux.sh   # 构建产物在 zmain(src-tauri/target/release/atray) → 101
# 101 首次需: sudo bash scripts/fix-kde-portal-qml.sh(仅 portal 通道需要)
```

运行环境注意：atray 强制 X11(GDK) 后端，deploy 脚本从图形会话进程继承
DISPLAY/XAUTHORITY/XDG_CURRENT_DESKTOP（XAUTHORITY 缺失会导致
"Failed to initialize GTK" panic）。

## 相关文件

- `src-tauri/src/kglobalaccel.rs` — KGA 直连后端（主攻方向，卡点所在）
- `src-tauri/src/portal.rs` — portal 后端（跨桌面备用）+ cleanup
- `src-tauri/src/lib.rs` — 后端分层选择、IPC、全屏 X11 强制
- `renderer/index.html` — 快捷键页 portal 模式适配（backend 字段）
- `scripts/fix-kde-portal-qml.sh` — Debian portal-kde 对话框修复
- `deploy-linux.sh` — 101 部署
