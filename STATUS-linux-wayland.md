# atray Linux Wayland 状态说明（交接文档）

> 2026-09-06 · 101（Debian 13 trixie + KDE Plasma 6.3, Wayland 会话, NVIDIA）
> 状态：**热键最终链路未通，已暂停**。本文给接手 agent 完整上下文。

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
| **按键触发** | ❌ | 注册/设键/持久化全通，但**按下快捷键不触发回调**（globalShortcutPressed 信号未达 / 未分发）——当前唯一卡点 |

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

## 未解决卡点（接手从这里开始）

**按键不触发**。已排除：
- 键确实注册（kglobalacceld 内存 + kglobalshortcutsrc 都有）
- 订阅确实注册（zbus fdo AddMatch 成功）
- 组件/动作名匹配

排查方向（按可能性）：
1. **先确认 kglobalacceld 是否发信号**：`dbus-monitor "interface='org.kde.kglobalaccel.Component'"`
   或 gdbus monitor，物理按键看有无 `globalShortcutPressed`。
   若无 → kglobalacceld 没抓键/没发（KGA 客户端状态机问题，见下）；
   若有 → atray 侧接收/分发问题。
2. KGlobalAccel 框架客户端与 daemon 之间可能有未复刻的状态机
   （daemon 源码 plasma/kglobalacceld：`src/globalshortcutsregistry.cpp`，
   keyEvent→GlobalShortcutsRegistry；组件需 `setIsPresent`？KCM 里动作的
   "present" 状态影响抓键）。观察 KWin 日志：
   `QT_LOGGING_RULES="kf.kglobalaccel.debug=true"` 或 journalctl 盯 kwin。
3. 备选验证 portal 通道在干净发行版（Fedora/KDE neon 等 portal-kde ≥6.7）
   是否可用——若可用，KDE 也可走 portal（省掉 KGA 客户端状态机问题），
   KGA 模块留作参考/备胎。
4. 兜底产品方案：应用不自行抓键，热键由 KDE 系统设置 → 自定义快捷键绑定
   `atray-ctl` 命令（D-Bus/Unix socket 通知应用 toggle）——100% 可靠但体验降级。

## 踩坑全集（写代码前必读）

详见 `src-tauri/src/kglobalaccel.rs` 文件头（完整 9 条 + 验证命令），摘要：

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
6. **setShortcut 需 flags=0x4 (NoAutoloading)**：否则 daemon 走 autoload 分支
   忽略新键（返回 [0]）；busctl 调它报参数错误，用 gdbus/zbus
7. **键编码 = Qt QKeySequence int**（mods|keycode）：Alt+Shift+A=167772225
8. **allShortcutInfos 签名 a(ssssssaiai)=6串+ai+ai**（不是 aai），反序列化错
   则 cleanup 静默失效 → 组件堆积
9. **zbus5 API 与 zbus4 不同**：receive_signal 泛型语义、add_match_rule 在
   fdo::DBusProxy、MatchRule::builder 部分方法返回 Result、MessageStream
   需规则才收广播

## 验证过的命令（可直接用）

```bash
# 组件/动作/键查询
busctl --user call org.kde.kglobalaccel /kglobalaccel org.kde.KGlobalAccel allComponents
busctl --user call org.kde.kglobalaccel /component/atray org.kde.kglobalaccel.Component allShortcutInfos

# 设键（flags=4 必须）
gdbus call --session --dest org.kde.kglobalaccel --object-path /kglobalaccel \
  --method org.kde.KGlobalAccel.setShortcut \
  "[\"atray\",\"toggle\",\"atray\",\"呼出/隐藏 atray 覆盖层\"]" "[167772225]" "4"

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
