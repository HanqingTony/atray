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
}

// 注意：与 anm（Alt+Shift+Z）可同机并存，故用不同默认值
const DEFAULT_HOTKEY: &str = "Alt+Shift+A";

impl Default for AppState {
    fn default() -> Self {
        let cfg = load_config();
        Self {
            hotkey: Mutex::new(Some(cfg.hotkey.unwrap_or_else(|| DEFAULT_HOTKEY.to_string()))),
            hotkey_shortcut: Mutex::new(None),
            plugin_shortcuts: Mutex::new(Vec::new()),
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
        }
    }
    save_config(&save);
    ok(serde_json::json!({ "ok": true, "plugins": save.plugins.len() }))
}

/// 读取当前配置（前端显示用）。
#[tauri::command]
fn atray_get_config(state: State<AppState>) -> IpcResp {
    ok(serde_json::json!({
        "hotkey": state.hotkey.lock().unwrap().clone().unwrap_or_else(|| DEFAULT_HOTKEY.to_string()),
        "plugins": load_config().plugins,
    }))
}

/// 重设全局快捷键：取消旧的，注册新的，持久化。
#[tauri::command]
fn atray_set_hotkey(app: tauri::AppHandle, state: State<AppState>, keys: String) -> IpcResp {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let keys = keys.trim().to_string();
    if keys.is_empty() {
        return err("快捷键不能为空".into());
    }
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

// ---------------------------------------------------------------------------
// 窗口显隐
// ---------------------------------------------------------------------------

fn show_main(win: &tauri::WebviewWindow) {
    let _ = win.show();
    let _ = win.set_focus();
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
    let single_instance = tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.show();
            let _ = win.set_focus();
        }
    });

    tauri::Builder::default()
        .plugin(single_instance)
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
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
            atray_hide
        ])
        .setup(|app| {
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
            }

            // 插件快捷键（配置里带 hotkey 的插件）
            sync_plugin_shortcuts(app.handle(), &load_config().plugins);

            // 全局热键（配置的或默认）
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let hk = app
                .state::<AppState>()
                .hotkey
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());
            let sc = tauri_plugin_global_shortcut::Shortcut::from_str(&hk).unwrap_or_else(|_| {
                tauri_plugin_global_shortcut::Shortcut::from_str(DEFAULT_HOTKEY).unwrap()
            });
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

            // 系统托盘：显示 / 设置 / 退出
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
            let show_i = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &settings_i, &quit_i])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/anm.ico"))
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
