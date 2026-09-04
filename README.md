# atray

WebUI 薄壳：把任意 web 应用（插件）嵌进全屏覆盖层，快捷键呼出/切换。

从 anotemanager 的 anm-tauri 薄壳分出：**无任何笔记/服务器概念**（与 anm-core 无关），
专门用于连接 webui。anm 那边保留笔记功能不动。

## 功能

- 全屏半透明覆盖层（原生 fullscreen 覆盖任务栏 + force-device-scale-factor=1）
- **插件**：设置菜单里输入 URL 添加 webui（http(s):// 或本地协议），底部工具栏按钮切换
- **快捷键**：全局热键（默认 Alt+Shift+Z）呼出/隐藏；每个插件可设独立快捷键直接呼出
- 托盘：显示 / 设置 / 退出

## 构建

- 系统依赖（Linux）：libwebkit2gtk-4.1-dev、libgtk-3-dev、libayatana-appindicator3-dev 等
- Windows（交叉）：`cargo build --release --target x86_64-pc-windows-gnu`，部署时
  `WebView2Loader.dll` 必须与 exe 同目录
- Linux：`cargo build --release`

## 部署（101 Windows）

```
bash deploy.sh            # exe + dll + renderer → C:\Users\<user>\atray\ + 桌面副本
bash deploy-front.sh      # 只推前端（零编译迭代）
bash deploy-linux.sh      # Linux 目标
```

配置持久化：`%APPDATA%/atray/config.json`（hotkey + plugins）。

## 插件机制

- 插件注册进 config.json（设置菜单管理，不再内置）
- 同源 iframe（atray://localhost 或 http://atray.localhost）→ 插件可直接用 `__TAURI__` 注入桥
- 跨源 iframe：纯展示（隔离）
