// Linux Wayland 全局热键：XDG Desktop Portal GlobalShortcuts 后端。
//
// 背景：Tauri 的 global-shortcut 插件在 Linux 走 X11 XGrabKey，Wayland 会话下
// 不可用。Wayland 生态唯一的跨桌面标准是 xdg-desktop-portal 的
// org.freedesktop.portal.GlobalShortcuts（KDE Plasma 6 / GNOME 45+ 等均实现）。
// 本模块用 ashpd 实现该后端：
//   - 建 session → BindShortcuts（系统对话框让用户确认/修改按键）
//   - 监听 Activated 信号分发：toggle（呼出/隐藏）/ plugin:<id>（切换应用）
//   - 配置变更（notify）→ 关闭旧 session → 重新绑定（再次弹系统对话框）
//
// 使用条件：Linux + Wayland 会话。Windows / X11 走 native（tauri 插件）不受影响。

#![cfg(target_os = "linux")]

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut, Shortcut};
use ashpd::desktop::Session;
use ashpd::zbus;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

use crate::{AppState, TrayConfig, hide_main, load_effective_config, show_main};

/// 是否应走 portal 后端（Linux + Wayland 会话）。
pub fn should_use_portal() -> bool {
    std::env::var("WAYLAND_DISPLAY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v == "wayland")
            .unwrap_or(false)
}

/// 启动 portal 热键后端。
/// 返回通知句柄：配置变更（快捷键增删改）时 `send(true)` 触发重绑。
pub fn spawn(app: AppHandle) -> watch::Sender<bool> {
    let (tx, rx) = watch::channel(false);
    tauri::async_runtime::spawn(portal_loop(app.clone(), rx));
    tauri::async_runtime::spawn(signal_listener(app));
    tx
}

/// 清理历史遗留的同应用快捷键注册（进程异常退出/升级重启会残留：
/// 组件名 token_ashpd_*，同名快捷键堆积会导致 KGlobalAccel 冲突、热键失效）。
/// 精确匹配：动作描述含「atray 覆盖层」（本应用写死的唯一标识），不误伤
/// 其它走 portal 的应用（chromium 等同样建 token_ashpd_* 组件）。
async fn cleanup_stale_atray(conn: &zbus::Connection) {
    use ashpd::zbus::zvariant::OwnedObjectPath;
    let msg = match conn
        .call_method(
            Some("org.kde.kglobalaccel"),
            "/kglobalaccel",
            Some("org.kde.KGlobalAccel"),
            "allComponents",
            &(),
        )
        .await
    {
        Ok(m) => m,
        Err(_) => return,
    };
    let comps: Vec<OwnedObjectPath> = match msg.body().deserialize() {
        Ok(v) => v,
        Err(_) => return,
    };
    for comp in comps {
        let name = match comp.as_str().rsplit('/').next() {
            Some(n) if n.starts_with("token_ashpd_") => n.to_string(),
            _ => continue,
        };
        // 读动作描述（allShortcutInfos：6 字符串 + 键组），判断是否本应用
        let msg = match conn
            .call_method(
                Some("org.kde.kglobalaccel"),
                comp.as_str(),
                Some("org.kde.kglobalaccel.Component"),
                "allShortcutInfos",
                &(),
            )
            .await
        {
            Ok(m) => m,
            Err(_) => continue,
        };
        let infos: Result<
            Vec<(String, String, String, String, String, String, Vec<Vec<i32>>)>,
            _,
        > = msg.body().deserialize();
        let mine = match infos {
            Ok(list) => list
                .iter()
                .any(|(_, friendly, _, _, _, _, _)| friendly.contains("atray 覆盖层")),
            Err(_) => false,
        };
        if !mine {
            continue;
        }
        // 注销该组件全部动作（组件对象残留无害，动作清掉即可解除占用）
        let msg = match conn
            .call_method(
                Some("org.kde.kglobalaccel"),
                comp.as_str(),
                Some("org.kde.kglobalaccel.Component"),
                "shortcutNames",
                &(),
            )
            .await
        {
            Ok(m) => m,
            Err(_) => continue,
        };
        let acts: Vec<String> = match msg.body().deserialize() {
            Ok(v) => v,
            Err(_) => continue,
        };
        for act in acts {
            let _ = conn
                .call_method(
                    Some("org.kde.kglobalaccel"),
                    "/kglobalaccel",
                    Some("org.kde.KGlobalAccel"),
                    "unregister",
                    &(name.clone(), act),
                )
                .await;
        }
        eprintln!("atray: 已清理残留快捷键组件 {name}");
    }
}

/// 主循环：清理历史残留 → 绑定当前配置 → 等待变更通知 → 关旧 session → 重绑。
async fn portal_loop(app: AppHandle, mut rx: watch::Receiver<bool>) {
    let mut current: Option<Session<GlobalShortcuts>> = None;
    let mut cleaned = false;
    loop {
        if !cleaned {
            // 启动时清理历史异常退出遗留的同应用快捷键（防冲突堆积）
            if let Ok(conn) = ashpd::zbus::Connection::session().await {
                cleanup_stale_atray(&conn).await;
            }
            cleaned = true;
        }
        // 关闭上一轮 session（解除其快捷键占用，否则改键后旧键仍生效）
        if let Some(old) = current.take() {
            let _ = old.close().await;
        }
        let cfg = load_effective_config();
        match bind_session(&app, &cfg).await {
            Ok(Some(session)) => {
                current = Some(session);
                // 绑定成功：挂起等待配置变更（不要定时重绑——会反复弹系统对话框）
                let _ = rx.changed().await;
            }
            Ok(None) => {
                // 没有需要注册的快捷键：等配置变更
                eprintln!("atray: 当前无快捷键可注册（portal），等待配置变更…");
                let _ = rx.changed().await;
            }
            Err(e) => {
                eprintln!("atray: portal 绑定失败: {e}（20 秒后重试）");
                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            }
        }
    }
}

/// 用当前配置创建一个绑定会话；用户未配置任何快捷键时返回 Ok(None)。
async fn bind_session(
    app: &AppHandle,
    cfg: &TrayConfig,
) -> Result<Option<Session<GlobalShortcuts>>, String> {
    let gs = GlobalShortcuts::new()
        .await
        .map_err(|e| format!("portal 服务不可用: {e}"))?;

    let mut list: Vec<NewShortcut> = Vec::new();
    if let Some(hk) = &cfg.hotkey {
        let mut sc = NewShortcut::new("toggle", "呼出/隐藏 atray 覆盖层");
        if let Some(t) = to_xdg_trigger(hk) {
            sc = sc.preferred_trigger(t.as_str());
        }
        list.push(sc);
    }
    for p in &cfg.plugins {
        if let Some(hk) = &p.hotkey {
            let name = if p.name.is_empty() { p.id.clone() } else { p.name.clone() };
            let mut sc = NewShortcut::new(format!("plugin:{}", p.id), format!("切换到 {name}"));
            if let Some(t) = to_xdg_trigger(hk) {
                sc = sc.preferred_trigger(t.as_str());
            }
            list.push(sc);
        }
    }
    if list.is_empty() {
        store_bound_keys(app, &[]);
        return Ok(None);
    }

    let session = gs
        .create_session(Default::default())
        .await
        .map_err(|e| format!("创建 session 失败: {e}"))?;
    let request = gs
        .bind_shortcuts(&session, &list, None, Default::default())
        .await
        .map_err(|e| format!("发起绑定失败: {e}"))?;
    // 等待用户在系统对话框确认/修改（可能被取消）
    let bound = request
        .response()
        .map_err(|e| format!("绑定被取消或失败: {e}"))?;
    store_bound_keys(app, bound.shortcuts());
    Ok(Some(session))
}

/// 把绑定结果（系统实际分配的按键描述）存 AppState，供前端显示。
fn store_bound_keys(app: &AppHandle, shortcuts: &[Shortcut]) {
    if let Some(state) = app.try_state::<AppState>() {
        let mut keys = state.portal_keys.lock().unwrap();
        keys.clear();
        for s in shortcuts {
            keys.push((s.id().to_string(), s.trigger_description().to_string()));
        }
    }
}

/// 监听 Activated 信号并分发（常驻；session 重建不中断，按 shortcut_id 分发即可）。
async fn signal_listener(app: AppHandle) {
    use futures_util::StreamExt;
    let gs = match GlobalShortcuts::new().await {
        Ok(g) => g,
        Err(e) => {
            eprintln!("atray: portal 信号监听不可用: {e}");
            return;
        }
    };
    let mut stream = match gs.receive_activated().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("atray: portal Activated 信号订阅失败: {e}");
            return;
        }
    };
    while let Some(act) = stream.next().await {
        let id = act.shortcut_id().to_string();
        dispatch(&app, &id);
    }
}

/// 快捷键分发（与 native 后端回调语义一致）。
fn dispatch(app: &AppHandle, id: &str) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if id == "toggle" {
        if win.is_visible().unwrap_or(false) {
            hide_main(&win);
        } else {
            show_main(&win);
        }
    } else if let Some(pid) = id.strip_prefix("plugin:") {
        show_main(&win);
        let _ = app.emit("atray-plugin-activate", pid.to_string());
    }
}

/// 把 atray 快捷键字符串（如 "Alt+Shift+A"）转 XDG shortcuts-spec 触发串
/// （如 "ALT+SHIFT+a"，xkb keysym 命名）。无法表达时返回 None（不设偏好键，
/// 让用户在系统对话框里自行设置）。
pub(crate) fn to_xdg_trigger(hk: &str) -> Option<String> {
    let mut parts: Vec<&str> = hk.split('+').collect();
    if parts.is_empty() {
        return None;
    }
    let key = parts.pop()?;
    let mut out: Vec<String> = Vec::new();
    for m in parts {
        let ml = m.to_ascii_lowercase();
        let mods = match ml.as_str() {
            "alt" => "ALT",
            "ctrl" | "control" => "CTRL",
            "shift" => "SHIFT",
            "super" | "cmd" | "meta" | "logo" | "win" => "LOGO",
            "num" => "NUM",
            _ => return None,
        };
        out.push(mods.to_string());
    }
    if out.is_empty() {
        return None; // 无修饰键的全局热键不自动绑定（防误设，弹窗里用户可自设）
    }
    // 主键 → xkb keysym 名（去掉 XKB_KEY_ 前缀；字母小写、F 键/数字保持）
    let k = if key.len() == 1 && key.chars().next()?.is_ascii_alphabetic() {
        key.to_ascii_lowercase()
    } else if key.len() == 1 && key.chars().next()?.is_ascii_digit() {
        key.to_string()
    } else if key.len() >= 2 && key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit())
    {
        key.to_string()
    } else {
        // 特殊键（前端只会产出字母/数字/F 键，这里兜底常见名）
        match key {
            "Enter" | "Return" => "Return".to_string(),
            "Escape" | "Esc" => "Escape".to_string(),
            "Tab" => "Tab".to_string(),
            "Space" => "space".to_string(),
            "Backspace" => "BackSpace".to_string(),
            "Delete" => "Delete".to_string(),
            "Insert" => "Insert".to_string(),
            "Home" => "Home".to_string(),
            "End" => "End".to_string(),
            "PageUp" => "Page_Up".to_string(),
            "PageDown" => "Page_Down".to_string(),
            "Up" => "Up".to_string(),
            "Down" => "Down".to_string(),
            "Left" => "Left".to_string(),
            "Right" => "Right".to_string(),
            _ => return None,
        }
    };
    out.push(k);
    Some(out.join("+"))
}
