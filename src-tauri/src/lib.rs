// atray：WebUI 薄壳（从 anm-tauri 分出的独立项目）。
//
// 职责：把任意 web 应用（插件）嵌进全屏覆盖层，快捷键呼出/切换。
//  - 透明全屏置顶窗口（原生 fullscreen + force-device-scale-factor=1）
//  - 全局热键（默认 Alt+Shift+Z）+ 每个插件可设独立快捷键直接呼出
//  - 系统托盘（显示 / 设置 / 退出）
//  - 插件注册持久化：config.json（hotkey + plugins）
//  - 自定义协议 atray://（Windows http://atray.localhost）服务 exe 旁 renderer/
//  - 无任何笔记/服务器概念（与 anm-core 无关）

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

#[cfg(target_os = "linux")]
mod portal;

/// 热键后端：native = tauri 插件（Windows / X11）；portal = XDG Desktop Portal
/// GlobalShortcuts（Linux Wayland，跨 KDE/GNOME 等桌面）。
#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Native,
    Portal,
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// 插件注册（设置菜单管理：URL 引入）。
#[derive(Serialize, Deserialize, Default, Clone)]
struct PluginCfg {
    id: String,
    name: String,
    url: String,
    /// 插件页面快捷键（可选，如 "Alt+Shift+1"）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hotkey: Option<String>,
}

/// 持久化配置（%APPDATA%/atray/config.json 或 ~/.config/atray/）。
#[derive(Serialize, Deserialize, Default, Clone)]
struct TrayConfig {
    hotkey: Option<String>,
    #[serde(default)]
    plugins: Vec<PluginCfg>,
}

fn config_path() -> Option<std::path::PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(std::path::PathBuf::from(appdata).join("atray").join("config.json"));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("atray").join("config.json"))
}

fn load_config() -> TrayConfig {
    let Some(path) = config_path() else {
        return TrayConfig::default();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &TrayConfig) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(&path, text);
    }
}

struct AppState {
    /// 当前已注册的全局快捷键（字符串形式，与 config 一致）
    hotkey: Mutex<Option<String>>,
    /// 当前注册的 Shortcut（重设时先 unregister）
    hotkey_shortcut: Mutex<Option<tauri_plugin_global_shortcut::Shortcut>>,
    /// 插件快捷键：id -> Shortcut（注销/重设用）
    plugin_shortcuts: Mutex<Vec<(String, tauri_plugin_global_shortcut::Shortcut)>>,
    /// Alt+数字 序号切换（仅覆盖层激活时生效）：已注册的 Shortcut（顺序变更时整组重注册）
    alt_shortcuts: Mutex<Vec<tauri_plugin_global_shortcut::Shortcut>>,
    /// 热键后端（setup 时确定）
    backend: Mutex<Backend>,
    /// portal 后端配置变更通知（Some = 已启动 portal 后端）
    portal_notify: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    /// portal 绑定结果：系统实际分配的 (shortcut_id, trigger_description)，
    /// 供前端显示（native 后端为空）。
    portal_keys: Mutex<Vec<(String, String)>>,
}

/// 配置变更后通知 portal 后端重绑（native 后端为空操作）。
fn notify_portal(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let guard = state.portal_notify.lock().unwrap();
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(true);
    }
}

// 注意：与 anm（Alt+Shift+Z）可同机并存，故用不同默认值
const DEFAULT_HOTKEY: &str = "Alt+Shift+A";

/// 读取「生效配置」：与 AppState::default 同一套默认值语义——
/// 配置文件不存在或从未写过 hotkey 键时，总快捷键用默认 Alt+Shift+A；
/// 配置文件出现 hotkey 键即尊重其值（含 null = 用户显式删除）。
fn load_effective_config() -> TrayConfig {
    let mut cfg = load_config();
    let has_hotkey_key = match config_path().filter(|p| p.exists()) {
        Some(p) => std::fs::read_to_string(&p)
            .map(|t| t.contains("\"hotkey\""))
            .unwrap_or(false),
        None => false,
    };
    if !has_hotkey_key {
        cfg.hotkey = Some(DEFAULT_HOTKEY.to_string());
    }
    cfg
}

impl Default for AppState {
    fn default() -> Self {
        let cfg = load_effective_config();
        Self {
            hotkey: Mutex::new(cfg.hotkey),
            hotkey_shortcut: Mutex::new(None),
            plugin_shortcuts: Mutex::new(Vec::new()),
            alt_shortcuts: Mutex::new(Vec::new()),
            backend: Mutex::new(Backend::Native),
            portal_notify: Mutex::new(None),
            portal_keys: Mutex::new(Vec::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct IpcResp {
    status: &'static str,
    data: serde_json::Value,
}

fn ok(data: serde_json::Value) -> IpcResp {
    IpcResp { status: "ok", data }
}
fn err(msg: String) -> IpcResp {
    IpcResp {
        status: "error",
        data: serde_json::Value::String(msg),
    }
}

/// 设置插件列表（整体替换）并同步快捷键。
#[tauri::command]
fn atray_set_config(app: tauri::AppHandle, cfg: Option<serde_json::Value>) -> IpcResp {
    let mut save = load_config();
    if let Some(cfg) = cfg {
        if let Some(plugins) = cfg.get("plugins").and_then(|v| v.as_array()) {
            let list: Vec<PluginCfg> = plugins
                .iter()
                .filter_map(|p| serde_json::from_value(p.clone()).ok())
                .collect();
            save.plugins = list.clone();
            sync_plugin_shortcuts(&app, &list);
            sync_alt_shortcuts(&app, &list); // 增删/排序后 Alt+数字 序号映射同步
        }
    }
    save_config(&save);
    notify_portal(&app); // portal 模式：插件/快捷键增删 → 重绑（native 为空操作）
    ok(serde_json::json!({ "ok": true, "plugins": save.plugins.len() }))
}

/// 读取当前配置（前端显示用）。
#[tauri::command]
fn atray_get_config(state: State<AppState>) -> IpcResp {
    let backend = if *state.backend.lock().unwrap() == Backend::Portal {
        "portal"
    } else {
        "native"
    };
    let portal_keys: serde_json::Map<String, serde_json::Value> = state
        .portal_keys
        .lock()
        .unwrap()
        .iter()
        .map(|(id, desc)| (id.clone(), serde_json::Value::String(desc.clone())))
        .collect();
    ok(serde_json::json!({
        "hotkey": state.hotkey.lock().unwrap().clone(),
        "plugins": load_config().plugins,
        "backend": backend,
        "portal_keys": portal_keys,
    }))
}

/// 重设全局快捷键：取消旧的，注册新的，持久化。
/// portal 模式（Wayland）：保存配置并通知重绑——由系统对话框确认实际按键。
#[tauri::command]
fn atray_set_hotkey(app: tauri::AppHandle, state: State<AppState>, keys: String) -> IpcResp {
    let keys = keys.trim().to_string();
    if keys.is_empty() {
        return err("快捷键不能为空".into());
    }
    // portal 模式：先校验字符串可被表达，然后只存配置，交给 portal 重绑弹窗
    if *state.backend.lock().unwrap() == Backend::Portal {
        #[cfg(target_os = "linux")]
        if portal::to_xdg_trigger(&keys).is_none() {
            return err(format!("该组合无法在 Wayland 全局热键中表达: {keys}（请用 Alt/Ctrl/Shift + 字母/数字/F 键）"));
        }
        *state.hotkey.lock().unwrap() = Some(keys.clone());
        let mut cfg = load_config();
        cfg.hotkey = Some(keys.clone());
        save_config(&cfg);
        notify_portal(&app);
        return ok(serde_json::json!({ "ok": true, "hotkey": keys }));
    }
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let sc = match tauri_plugin_global_shortcut::Shortcut::from_str(&keys) {
        Ok(s) => s,
        Err(e) => return err(format!("无法解析快捷键 {keys}: {e}")),
    };
    if let Some(old) = state.hotkey_shortcut.lock().unwrap().take() {
        let _ = app.global_shortcut().unregister(old);
    }
    let hk = keys.clone();
    let reg = app.global_shortcut().on_shortcut(sc.clone(), |app, _sc, event| {
        use tauri_plugin_global_shortcut::ShortcutState;
        if event.state == ShortcutState::Pressed {
            if let Some(win) = app.get_webview_window("main") {
                if win.is_visible().unwrap_or(false) {
                    hide_main(&win);
                } else {
                    show_main(&win);
                }
            }
        }
    });
    match reg {
        Ok(()) => {
            *state.hotkey.lock().unwrap() = Some(hk.clone());
            *state.hotkey_shortcut.lock().unwrap() = Some(sc);
            let mut cfg = load_config();
            cfg.hotkey = Some(hk);
            save_config(&cfg);
            ok(serde_json::json!({ "ok": true, "hotkey": keys }))
        }
        Err(e) => err(format!("注册快捷键失败: {e}")),
    }
}

/// 删除总快捷键：注销已注册的全局热键并持久化 null（托盘仍可呼出/退出）。
/// portal 模式：清配置并重绑（弹窗里不再包含总快捷键）。
#[tauri::command]
fn atray_clear_hotkey(app: tauri::AppHandle, state: State<AppState>) -> IpcResp {
    if *state.backend.lock().unwrap() == Backend::Portal {
        *state.hotkey.lock().unwrap() = None;
        let mut cfg = load_config();
        cfg.hotkey = None;
        save_config(&cfg);
        notify_portal(&app);
        return ok(serde_json::json!({ "ok": true, "hotkey": null }));
    }
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    if let Some(old) = state.hotkey_shortcut.lock().unwrap().take() {
        let _ = app.global_shortcut().unregister(old);
    }
    *state.hotkey.lock().unwrap() = None;
    let mut cfg = load_config();
    cfg.hotkey = None;
    save_config(&cfg);
    ok(serde_json::json!({ "ok": true, "hotkey": null }))
}

/// 读取开机自启状态（注册表 Run 键）。
#[tauri::command]
fn atray_get_autostart(app: tauri::AppHandle) -> IpcResp {
    use tauri_plugin_autostart::ManagerExt;
    match app.autolaunch().is_enabled() {
        Ok(enabled) => ok(serde_json::json!({ "enabled": enabled })),
        Err(e) => err(format!("读取开机启动状态失败: {e}")),
    }
}

/// 设置/取消开机自启（写入/删除 HKCU Run 键；自启带 --autostart 参数，隐藏启动）。
#[tauri::command]
fn atray_set_autostart(app: tauri::AppHandle, enable: bool) -> IpcResp {
    use tauri_plugin_autostart::ManagerExt;
    let r = if enable {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    match r {
        Ok(()) => ok(serde_json::json!({ "enabled": enable })),
        Err(e) => err(format!("设置开机启动失败: {e}")),
    }
}

/// 退出应用（右上角小按钮用，与托盘「退出」一致）。
#[tauri::command]
fn atray_quit(app: tauri::AppHandle) -> IpcResp {
    app.exit(0);
    ok(serde_json::json!({ "ok": true }))
}

/// 隐藏覆盖层（前端点击空白取消激活用）。
#[tauri::command]
fn atray_hide(app: tauri::AppHandle) -> IpcResp {
    if let Some(win) = app.get_webview_window("main") {
        hide_main(&win);
    }
    ok(serde_json::json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// 插件快捷键
// ---------------------------------------------------------------------------

/// 同步插件快捷键：注销被删/无热键的，注册新增/更新的（按下 = 显示窗口 + 通知前端激活插件）
fn sync_plugin_shortcuts(app: &tauri::AppHandle, plugins: &[PluginCfg]) {
    // portal 模式：快捷键由 portal 统一绑定（见 portal.rs），native 注册跳过
    if let Some(state) = app.try_state::<AppState>() {
        if *state.backend.lock().unwrap() == Backend::Portal {
            return;
        }
    }
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let state = app.state::<AppState>();
    let mut registered = state.plugin_shortcuts.lock().unwrap();
    registered.retain(|(id, sc)| {
        let keep = plugins.iter().any(|p| &p.id == id);
        if !keep {
            let _ = app.global_shortcut().unregister(sc.clone());
        }
        keep
    });
    for p in plugins {
        let existing = registered.iter().position(|(id, _)| id == &p.id);
        match (&p.hotkey, existing) {
            (Some(hk), Some(idx)) => {
                if let Ok(sc) = tauri_plugin_global_shortcut::Shortcut::from_str(hk) {
                    let (_, old) = registered.remove(idx);
                    let _ = app.global_shortcut().unregister(old);
                    let pid = p.id.clone();
                    let _ = app.global_shortcut().on_shortcut(sc.clone(), move |app, _sc, event| {
                        if event.state == ShortcutState::Pressed {
                            if let Some(win) = app.get_webview_window("main") {
                                show_main(&win);
                            }
                            let _ = app.emit("atray-plugin-activate", pid.clone());
                        }
                    });
                    registered.push((p.id.clone(), sc));
                }
            }
            (Some(hk), None) => {
                if let Ok(sc) = tauri_plugin_global_shortcut::Shortcut::from_str(hk) {
                    let pid = p.id.clone();
                    let _ = app.global_shortcut().on_shortcut(sc.clone(), move |app, _sc, event| {
                        if event.state == ShortcutState::Pressed {
                            if let Some(win) = app.get_webview_window("main") {
                                show_main(&win);
                            }
                            let _ = app.emit("atray-plugin-activate", pid.clone());
                        }
                    });
                    registered.push((p.id.clone(), sc));
                }
            }
            (None, Some(idx)) => {
                let (_, old) = registered.remove(idx);
                let _ = app.global_shortcut().unregister(old);
            }
            (None, None) => {}
        }
    }
}

/// Alt+数字（按列表顺序：Alt+1..Alt+9、Alt+0=第10个）快捷切换。
///
/// 需求：覆盖层激活（窗口可见且聚焦）时无论焦点在哪个 web 应用 iframe 里都能切换，
/// 纯前端 keydown 收不到跨源 iframe 内的按键，故走全局热键；但只在窗口激活时响应，
/// 窗口隐藏/失焦时按下不做任何事（不抢其他程序的 Alt+数字）。
fn sync_alt_shortcuts(app: &tauri::AppHandle, plugins: &[PluginCfg]) {
    // portal 模式：Alt+数字 由 portal 表达代价高（10 个系统动作 + 全局触发语义不符，
    // 该功能服务于窗口内切换），Wayland 下跳过；窗口内切换可用各应用自有快捷键
    if let Some(state) = app.try_state::<AppState>() {
        if *state.backend.lock().unwrap() == Backend::Portal {
            return;
        }
    }
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let state = app.state::<AppState>();
    let mut regs = state.alt_shortcuts.lock().unwrap();
    // 顺序/数量变化时整组注销重注册
    for sc in regs.drain(..) {
        let _ = app.global_shortcut().unregister(sc);
    }
    for (i, p) in plugins.iter().take(10).enumerate() {
        let key = if i == 9 { "Alt+0".into() } else { format!("Alt+{}", i + 1) };
        let Ok(sc) = tauri_plugin_global_shortcut::Shortcut::from_str(&key) else {
            continue;
        };
        let pid = p.id.clone();
        match app.global_shortcut().on_shortcut(sc.clone(), move |app, _sc, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let Some(win) = app.get_webview_window("main") else {
                return;
            };
            // 仅覆盖层激活时切换；隐藏或失焦一律忽略
            if !win.is_visible().unwrap_or(false) || !win.is_focused().unwrap_or(false) {
                return;
            }
            let _ = win.set_focus();
            let _ = app.emit("atray-plugin-activate", pid.clone());
        }) {
            Ok(()) => regs.push(sc),
            Err(e) => {
                // 被占用（如某应用自设了 Alt+1）：该序号不可用，其余照常
                eprintln!("Alt+数字注册失败 {key}（可能被占用）: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 窗口显隐
// ---------------------------------------------------------------------------

fn show_main(win: &tauri::WebviewWindow) {
    let _ = win.show();
    let _ = win.set_focus();
    // Wayland：窗口 hide 后重新 show，KWin 可能已退出全屏状态——每次呼出恢复全屏
    #[cfg(target_os = "linux")]
    {
        let _ = win.set_fullscreen(true);
    }
    let _ = win.emit("atray-event", "shown");
}
fn hide_main(win: &tauri::WebviewWindow) {
    let _ = win.hide();
    let _ = win.emit("atray-event", "hidden");
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

pub fn run() {
    #[cfg(target_os = "linux")]
    {
        // 强制 X11(GDK)后端:实测 tao/WebKitGTK 的 Wayland fullscreen 有几何 bug
        // (GTK 状态置 fullscreen 但 KWin 不给全屏尺寸,窗口保持 ~46% 大小;
        // 原生 GTK 测试正常 → 问题在 tao 层)。XWayland 下 X11 全屏可靠:
        // WebKitGTK 渲染、托盘、portal 全局热键(与窗口平台无关)均不受影响。
        // 注:tao 修复后可移除本行改回 Wayland 原生。
        std::env::set_var("GDK_BACKEND", "x11");
    }
    let single_instance = tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.show();
            let _ = win.set_focus();
        }
    });

    tauri::Builder::default()
        .plugin(single_instance)
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 开机自启：Run 键指向 exe 并带 --autostart（启动后隐藏，托盘待命）
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("atray")
                .arg("--autostart")
                .build(),
        )
        // 自定义协议 atray：把 exe 同目录 renderer/ 以平台对应形态提供
        // （Windows http://atray.localhost；Linux atray://localhost）。
        .register_uri_scheme_protocol("atray", |_ctx, request| {
            use tauri::http::{header, Response, StatusCode};
            let renderer = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("renderer")));
            let path = request.uri().path().trim_start_matches('/');
            let respond = |status: StatusCode, body: Vec<u8>, mime: &str| {
                Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CACHE_CONTROL, "no-cache")
                    .body::<std::borrow::Cow<'static, [u8]>>(body.into())
                    .unwrap()
            };
            match renderer {
                Some(dir) if !path.is_empty() && !path.contains("..") => {
                    let file = dir.join(path);
                    match std::fs::read(&file) {
                        Ok(bytes) => {
                            let mime = match file
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "html" => "text/html; charset=utf-8",
                                "js" => "text/javascript; charset=utf-8",
                                "css" => "text/css; charset=utf-8",
                                "png" => "image/png",
                                "ico" => "image/x-icon",
                                _ => "application/octet-stream",
                            };
                            respond(StatusCode::OK, bytes, mime)
                        }
                        Err(_) => respond(StatusCode::NOT_FOUND, Vec::new(), "text/plain"),
                    }
                }
                _ => respond(StatusCode::NOT_FOUND, Vec::new(), "text/plain"),
            }
        })
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            atray_set_config,
            atray_get_config,
            atray_set_hotkey,
            atray_clear_hotkey,
            atray_get_autostart,
            atray_set_autostart,
            atray_hide,
            atray_quit
        ])
        .setup(|app| {
            // 开机自启（带 --autostart）→ 隐藏启动，托盘待命，避免登录后全屏覆盖层弹出
            if std::env::args().any(|a| a == "--autostart") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
            // 外部前端：navigate 到自定义协议页面（平台 URL 形态不同）
            if let Some(win) = app.get_webview_window("main") {
                let exists = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("renderer").join("index.html")))
                    .map(|p| p.exists())
                    .unwrap_or(false);
                if exists {
                    let url_str = if cfg!(windows) {
                        "http://atray.localhost/index.html"
                    } else {
                        "atray://localhost/index.html"
                    };
                    if let Ok(url) = url_str.parse::<tauri::Url>() {
                        let _ = win.navigate(url);
                    }
                }
                // WebKitGTK/Wayland：全屏需等窗口 map 后才生效（创建期调用会丢）——
                // 延迟到窗口呈现后补一次；此后每次 show 也恢复全屏（见 show_main）
                #[cfg(target_os = "linux")]
                {
                    let w = win.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                        let r = w.set_fullscreen(true);
                        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                        let fs = w.is_fullscreen().unwrap_or(false);
                        let sz = w.outer_size().map(|s| format!("{s:?}")).unwrap_or_default();
                        let m = match w.current_monitor() {
                            Ok(Some(m)) => format!("{:?}", m.size()),
                            _ => "?".to_string(),
                        };
                        if !fs || sz != m {
                            eprintln!("atray: 全屏异常 set_fullscreen={r:?} is_fullscreen={fs} outer={sz} monitor={m}");
                        }
                    });
                }
            }

            // 热键后端选择：Linux Wayland 会话 → portal（XDG Desktop Portal
            // GlobalShortcuts，跨 KDE/GNOME 等桌面）；否则 native（tauri 插件，
            // Windows / X11）。注：XWayland 下 GTK 仍可能走 X11——portal 分支
            // 失败时托盘仍可用（见 portal.rs 日志）。
            let use_portal: bool = {
                #[cfg(target_os = "linux")]
                {
                    portal::should_use_portal()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    false
                }
            };
            *app.state::<AppState>().backend.lock().unwrap() = if use_portal {
                Backend::Portal
            } else {
                Backend::Native
            };

            if use_portal {
                // portal 后端：绑定 + 信号监听在异步任务中（首轮即绑定；
                // 此后配置变更由 IPC 侧 notify_portal 触发重绑）
                let tx = portal::spawn(app.handle().clone());
                *app.state::<AppState>().portal_notify.lock().unwrap() = Some(tx);
            } else {
                // 插件快捷键（配置里带 hotkey 的插件）+ Alt+数字 序号切换
                let plugins = load_config().plugins;
                sync_plugin_shortcuts(app.handle(), &plugins);
                sync_alt_shortcuts(app.handle(), &plugins);

                // 全局热键（配置的；显式删除后为 None → 不注册，托盘仍可用）
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let hk = app.state::<AppState>().hotkey.lock().unwrap().clone();
                if let Some(hk) = hk {
                    let sc = tauri_plugin_global_shortcut::Shortcut::from_str(&hk).unwrap_or_else(
                        |_| {
                            tauri_plugin_global_shortcut::Shortcut::from_str(DEFAULT_HOTKEY).unwrap()
                        },
                    );
                    match app.global_shortcut().on_shortcut(sc.clone(), |app, _sc, event| {
                        use tauri_plugin_global_shortcut::ShortcutState;
                        if event.state == ShortcutState::Pressed {
                            if let Some(win) = app.get_webview_window("main") {
                                if win.is_visible().unwrap_or(false) {
                                    hide_main(&win);
                                } else {
                                    show_main(&win);
                                }
                            }
                        }
                    }) {
                        Ok(()) => {
                            *app.state::<AppState>().hotkey_shortcut.lock().unwrap() = Some(sc);
                        }
                        Err(e) => {
                            // 热键被占用（如 anm 同机并存）：降级运行，托盘仍可呼出
                            eprintln!("全局热键注册失败（可能被占用），托盘仍可用: {e}");
                        }
                    }
                }
            }

            // 系统托盘：显示 / 设置 / 退出
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
            let show_i = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &settings_i, &quit_i])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/atray.ico"))
                .expect("图标解析失败");
            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .tooltip("atray")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(win) = app.get_webview_window("main") {
                            show_main(&win);
                        }
                    }
                    "settings" => {
                        if let Some(win) = app.get_webview_window("main") {
                            show_main(&win);
                        }
                        let _ = app.emit("atray-menu", "settings");
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            if win.is_visible().unwrap_or(false) {
                                hide_main(&win);
                            } else {
                                show_main(&win);
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri 运行失败");
}
