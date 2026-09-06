# atray

**WebUI 薄壳**：把任意 web 应用嵌进全屏覆盖层，用快捷键在应用间快速切换。

> **v1.2.0**（2026-09-06）：Linux Wayland 热键全链路打通（KGA SetPresent 根因修复 +
> KGA 对账 v2），Windows 功能集（快捷键删除/排序/开机自启/Alt+数字）已稳定。
> 从 anotemanager 的 anm-tauri 薄壳分出：**不含任何笔记/服务器概念**（与 anm-core 无关），
> 专门用于连接 webui（ComfyUI、各类面板、本地服务等）。

## 功能总览

### 覆盖层

- 全屏半透明压暗桌面（`transparent` 窗口 + CSS 遮罩），原生 fullscreen 覆盖任务栏
- Windows `--force-device-scale-factor=1`：物理=逻辑像素，根治 DPI 缩放（125% 缩放等）
- 无 web 应用时的空态：艺术大标题（渐变半透明）+ 「＋ 添加web应用」引导按钮

### web 应用（插件）

- **设置里输入 URL 注册**（`http(s)://` 远程 / `atray://localhost/…` 本地同源），iframe 嵌入
- **同源应用可直接调用 `__TAURI__`**（Rust 桥）；跨源应用纯展示（隔离）
- 底部工具栏按钮切换（贴边方框风格）；Esc 回主面板
- 应用列表持久化于 config.json（`%APPDATA%/atray/config.json` 或 `~/.config/atray/`）

### 快捷键体系

- **总快捷键**（默认 `Alt+Shift+Z`）：呼出/隐藏覆盖层
- **每个 web 应用独立快捷键**（如 `Alt+Shift+1`）：任意时刻按下直接切到对应应用（窗口自动呼出）
- 热键被占用时**降级运行**（托盘仍可呼出），不崩溃
- 设置 → 快捷键页：总 + 每个应用快捷键**集中可见可设**，按下组合**即时生效**（Esc 取消）

### 设置（菜单式，全部即时生效）

| 页 | 内容 |
|---|---|
| 快捷键 | 总快捷键 + 各应用快捷键（行内「设置」→ 按新组合即保存） |
| web 应用 | 列表（名称/URL/当前热键/删除）+ URL 添加 |
| 布局 | **隐藏按钮位置**（四角可选，贴角：仅朝向屏幕中心的角圆角）+ **菜单栏位置**（左下/右下/底部居中）；两者不能同角（互斥拦截） |

- 隐藏按钮（✕，默认右上贴角）：点击 = 隐藏覆盖层（快捷键语义）；真退出在托盘右键菜单
- 布局记忆于 localStorage

### 托盘

右键菜单：**显示 / 设置 / 退出**（退出 = 真退出进程）

## 使用速查

| 操作 | 方式 |
|---|---|
| 呼出/隐藏 | 总快捷键（Alt+Shift+Z）或托盘「显示」 |
| 隐藏（不退出） | 右上角 ✕ |
| 真退出 | 托盘右键 → 退出 |
| 添加 web 应用 | 空态「＋ 添加web应用」按钮 / ⚙ 设置 → web 应用 |
| 应用快捷键 | ⚙ 设置 → 快捷键 → 应用行「设置」→ 按组合 |
| 查看全部快捷键 | ⚙ 设置 → 快捷键页 |
| 调整按钮/菜单栏位置 | ⚙ 设置 → 布局 |

## 平台支持

| 平台 | 全屏覆盖层 | 全局热键 |
|---|---|---|
| Windows | 透明 + 原生全屏（WebView2） | tauri global-shortcut（native） |
| Linux X11 | 透明 + 全屏 | tauri global-shortcut（native） |
| Linux Wayland（KDE/GNOME/Hyprland） | 全屏（页面自带深色背景，无需窗口透明） | **XDG GlobalShortcuts portal**（跨桌面标准） |

### Wayland 热键说明（portal 后端）

- 探测：Wayland 会话自动启用 `portal` 后端（[ashpd](https://crates.io/crates/ashpd)
  实现 `org.freedesktop.portal.GlobalShortcuts`）；X11/Windows 走 native 注册
- 首次绑定弹**系统对话框**（由桌面提供）确认/修改按键；此后绑定持久化，重启不弹窗
- 配置变更（设置页改键/增删应用）→ 重绑（再次弹系统框）
- 设置页显示系统实际分配的按键（`portal_keys` 回填）；「Alt+数字 序号切换」为
  Windows/X11 功能，Wayland 下不注册
- **KDE 注意**：Debian 13 的 `xdg-desktop-portal-kde` 6.3.5 缺对话框 QML
  （上游发布缺陷，6.7+ 修复），需先跑 `scripts/fix-kde-portal-qml.sh`（补 ki18n
  qmldir + portal 对话框 QML，幂等）。否则绑定框弹不出、请求挂起
- 启动时自动清理历史异常退出残留的同应用快捷键（防 KGlobalAccel 冲突堆积）

## 技术

- **Tauri v2**：Rust 后端 + 纯 HTML 前端单文件（`renderer/index.html`），零构建
- 前端外部加载：exe 旁 `renderer/`，经自定义协议 `atray://`（Windows `http://atray.localhost`）
  提供——**改前端零编译**（`deploy-front.sh` 推送即生效）
- 应用快捷键：Rust 动态注册/注销全局热键 → 显示窗口 + emit `atray-plugin-activate` → 前端切换
- 调试：Windows 9223 端口（CDP）
- 图标：专属徽标（渐变 A，ico + png）

## 构建

系统依赖（Linux 编译）：`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`

```bash
cd src-tauri
cargo build --release                                   # Linux 原生
cargo build --release --target x86_64-pc-windows-gnu    # Windows 交叉编译
```

> 注意：交叉编译的 exe 导入表引用 `WebView2Loader.dll`，必须与 exe 同目录部署。

## 部署

```bash
bash deploy.sh           # Windows：exe + dll + renderer → C:\Users\<user>\atray\ + 桌面副本
bash deploy-front.sh     # 只推前端（零编译迭代）
bash deploy-linux.sh     # Linux 目标（~/atray/，自动探测 Wayland/X11 会话启动）
```

> Debian/KDE 首次部署：先 `sudo bash scripts/fix-kde-portal-qml.sh`（见平台支持节）。

## 与 anm 的关系

| 项目 | 定位 |
|---|---|
| **anotemanager**（anm） | 笔记系统：anm-core 服务 + anm-tauri 笔记覆盖层 + MCP |
| **atray**（本仓库） | 通用 WebUI 薄壳，与笔记无关，可同机并存（快捷键默认错开） |

## 版本历史

- **v1.2.0**（2026-09-06）：Linux Wayland 热键全链路打通——根因修复：KGA
  `setShortcut` 必须带 `SetPresent|NoAutoloading`（0x6），缺 SetPresent 动作不
  抓键（按键无信号；`Component.isActive()`=false 可诊断），物理按键呼出/隐藏
  实测通过；KGA 同步重构为 desired-state 对账 v2（config.json 唯一期望态，
  启动/IPC/漂移 watchdog 三触发点共用 reconcile_all，系统设置改键等外部偏离
  ≤20s 自动还原；watchdog 内容比较防自触发）；完整定位过程与诊断命令见
  `STATUS-linux-wayland.md`。另含 Windows 功能集：快捷键删除、web 应用排序、
  开机自启（通用页）、Alt+数字窗口内快速切换
- **v1.1.0**（2026-09-06）：Linux Wayland 支持——XDG GlobalShortcuts portal 热键后端
  （ashpd，跨 KDE/GNOME/Hyprland）+ 显式全屏 + 历史残留快捷键清理；
  新增 `scripts/fix-kde-portal-qml.sh`（Debian 13 portal-kde 对话框缺陷修复）
- **v1.0.0**（2026-09-04）：从 anm-tauri 分出独立项目；术语「web 应用」；设置 v2
  （快捷键集中可见/即时生效/单按钮）；布局配置（隐藏按钮四角 + 菜单栏位置，互斥同角）；
  贴角圆角收敛（仅朝中心角圆角）；专属图标；右上角 ✕ 隐藏按钮；文档完善

## 路线图

未来规划（v1.x 打磨清单 / v2.0.0 场景功能）见 [ROADMAP.md](ROADMAP.md)。
