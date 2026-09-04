# atray

**WebUI 薄壳**：把任意 web 应用嵌进全屏覆盖层，用快捷键在应用间快速切换。

从 anotemanager 的 anm-tauri 薄壳分出：**不含任何笔记/服务器概念**（与 anm-core 无关），
专门用于连接 webui（ComfyUI、各类面板、本地服务等）。anm 那边保留笔记功能不动。

## 设计

```
┌──────────────────────────────────────┐
│  全屏半透明覆盖层（原生 fullscreen）    │
│  ┌────────────────────────────────┐  │
│  │  web 应用 iframe（当前激活的）    │  │
│  └────────────────────────────────┘  │
│  [⚙ 设置][主面板][应用1][应用2]…       │ ← 底部工具栏（贴边方框）
└──────────────────────────────────────┘
```

- **覆盖层**：透明全屏压暗桌面（`transparent` + CSS 半透明），原生 fullscreen 覆盖任务栏；
  Windows 上 `--force-device-scale-factor=1` 根治 DPI 缩放（物理=逻辑像素）
- **web 应用**：设置里输入 URL 注册（`http(s)://` 远程，或 `atray://localhost/…` 本地同源），
  iframe 嵌入；**同源应用可直接调用 `__TAURI__`**（Rust 桥），跨源应用纯展示（隔离）
- **快捷键体系**：
  - **总快捷键**（默认 `Alt+Shift+A`，与 anm 的 Alt+Shift+Z 错开可同机并存）：呼出/隐藏覆盖层
  - **每个 web 应用独立快捷键**（如 `Alt+Shift+1`）：任意时刻按下直接切到该应用（窗口自动呼出）
  - 热键被占用时降级运行（托盘仍可呼出），不崩溃
- **托盘**：显示 / 设置 / 退出
- **配置**：`%APPDATA%/atray/config.json`（Windows）或 `~/.config/atray/`（Linux），
  保存总快捷键 + 应用列表（含各自快捷键），**所有设置改动即时生效**

## 使用

| 操作 | 方式 |
|---|---|
| 呼出/隐藏覆盖层 | 总快捷键（默认 Alt+Shift+A）或托盘「显示」 |
| 添加 web 应用 | ⚙ 设置 → web 应用 → 输入 URL（+可选名称）→ 添加 |
| 删除 web 应用 | ⚙ 设置 → web 应用 → 行内「删除」 |
| 设置总快捷键 | ⚙ 设置 → 快捷键 → 总快捷键行「设置」→ 按新组合（即时生效） |
| 设置应用快捷键 | ⚙ 设置 → 快捷键 → 该应用行「设置」→ 按新组合（即时生效，Esc 取消） |
| 查看当前快捷键 | ⚙ 设置 → 快捷键（总 + 每个应用全部可见） |
| 切换应用 | 底部按钮 / 应用快捷键 |
| 回主面板 | 底部「主面板」/ Esc |
| 隐藏窗口 | 主面板时按 Esc |

## 技术

- **Tauri v2**（Rust 后端 + 纯 HTML 前端单文件，零构建）
- 前端外部加载：exe 旁 `renderer/` 目录，经自定义协议 `atray://`（Windows 为
  `http://atray.localhost`）提供——**改前端零编译**（`deploy-front.sh` 推送即生效）
- 应用快捷键：Rust 动态注册/注销全局热键（按下 → 显示窗口 + emit `atray-plugin-activate` → 前端切换）

## 构建

系统依赖（Linux 编译）：`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`

```bash
# Linux（原生）
cd src-tauri && cargo build --release

# Windows（交叉编译）
cd src-tauri && cargo build --release --target x86_64-pc-windows-gnu
# 注意：交叉编译的 exe 导入表引用 WebView2Loader.dll，必须与 exe 同目录部署
```

## 部署

```bash
bash deploy.sh           # Windows：exe + dll + renderer → C:\Users\<user>\atray\ + 桌面副本
bash deploy-front.sh     # 只推前端（零编译迭代）
bash deploy-linux.sh     # Linux 目标（~/atray/，DISPLAY=:0 启动）
```

## 与 anm 的关系

- **anotemanager**（anm）：笔记系统（anm-core 服务 + anm-tauri 笔记覆盖层 + MCP）
- **atray**（本仓库）：通用 WebUI 薄壳，与笔记无关
- 两者可同机并存（快捷键默认错开）；后续如需，atray 可作为 anm 生态的通用面板层

## 开发日志

- 2026-09：初始版（从 anm-tauri 分出）；术语统一为「web 应用」；设置面板 v2
  （快捷键集中可见、即时生效、单「完成」按钮）
