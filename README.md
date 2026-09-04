# atray

**WebUI 薄壳**：把任意 web 应用嵌进全屏覆盖层，用快捷键在应用间快速切换。

> **v1.0.0**（2026-09）：功能稳定版。
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

- **总快捷键**（默认 `Alt+Shift+A`，与 anm 的 Alt+Shift+Z 错开可同机并存）：呼出/隐藏覆盖层
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
| 呼出/隐藏 | 总快捷键（Alt+Shift+A）或托盘「显示」 |
| 隐藏（不退出） | 右上角 ✕ |
| 真退出 | 托盘右键 → 退出 |
| 添加 web 应用 | 空态「＋ 添加web应用」按钮 / ⚙ 设置 → web 应用 |
| 应用快捷键 | ⚙ 设置 → 快捷键 → 应用行「设置」→ 按组合 |
| 查看全部快捷键 | ⚙ 设置 → 快捷键页 |
| 调整按钮/菜单栏位置 | ⚙ 设置 → 布局 |

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
bash deploy-linux.sh     # Linux 目标（~/atray/，DISPLAY=:0 启动）
```

## 与 anm 的关系

| 项目 | 定位 |
|---|---|
| **anotemanager**（anm） | 笔记系统：anm-core 服务 + anm-tauri 笔记覆盖层 + MCP |
| **atray**（本仓库） | 通用 WebUI 薄壳，与笔记无关，可同机并存（快捷键默认错开） |

## 版本历史

- **v1.0.0**（2026-09-04）：从 anm-tauri 分出独立项目；术语「web 应用」；设置 v2
  （快捷键集中可见/即时生效/单按钮）；布局配置（隐藏按钮四角 + 菜单栏位置，互斥同角）；
  贴角圆角收敛（仅朝中心角圆角）；专属图标；右上角 ✕ 隐藏按钮；文档完善
