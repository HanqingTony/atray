// KDE KGlobalAccel 直连热键后端（Wayland/KDE，固定组件幂等）。
//
// ## 为什么有本模块（完整踩坑史，2026-09，101 KDE Wayland 实测）
//
// 需求演化：先是「Wayland 全局热键」→ portal 通道 → 用户要求「幂等：系统里
// 维护一份设置，已有则复用不再新增」。
//
// ### 坑 1：Tauri global-shortcut 插件在 Wayland 不可用
// Linux 实现走 X11 XGrabKey；Wayland 会话下 GTK 走 wayland 后端则完全失效。
//
// ### 坑 2：portal 通道不幂等（本模块存在的根本原因）
// xdg-desktop-portal-kde 的绑定以 session 为粒度，组件名 = token_ashpd_<随机>，
// 每次启动/重绑都新建组件并把键写进 kglobalshortcutsrc。异常退出（pkill/升级）
// 残留组件**不清理**，多次重启后多个组件同绑一键 → KGlobalAccel 冲突 →
// 热键全部失效（表现为「按了没反应」）。portal 侧无「固定组件」概念，只能事后
// cleanup（见 portal.rs cleanup_stale_atray），不优雅。
//
// ### 坑 3：Debian 13 的 portal-kde 6.3.5 绑定时无确认对话框
// 上游 6.3.5 发布包缺一批对话框 QML（GlobalShortcutsDialog 等 2025 年新增文件
// 未进 tarball，CMakeLists 却引用了；二进制 qrc 里只有 UserInfoDialog.qml）。
// 表现：BindShortcuts 到达后端、无任何日志、QuickDialog rootObjects 为空静默
// return、请求永久挂起（atray 侧 response 永不返回）。修复脚本：
// scripts/fix-kde-portal-qml.sh（补 org.kde.ki18n qmldir + 从上游 master 装
// portal 对话框 QML 到 /usr/lib/.../qt6/qml/org/kde/xdgdesktopportal/）。
// 教训：怀疑「对话框没弹」时先 strings 二进制查 qrc 资源，再查 import 依赖。
//
// ### 坑 4：tao/WebKitGTK 的 Wayland fullscreen 有几何 bug
// set_fullscreen 返回 Ok、GTK is_fullscreen=true，但 KWin 不给全屏几何
// （实测窗口 1780x1380 而屏幕 3840x2160@缩放；原生 GTK3 测试全屏正常 →
// 问题在 tao 层）。解法：Linux 强制 GDK_BACKEND=x11（XWayland 下 X11 全屏
// 可靠；portal/KGA 热键走 session bus 与窗口平台无关，不受影响）。见 lib.rs
// run() 开头的注释。tao 修复后可删。
//
// ### 坑 5：KGlobalAccel D-Bus 的 actionId 是 4 元素数组
// [组件唯一名, 动作唯一名, 组件友好名, 动作友好名]（KGlobalAccel::actionIdFields
// 枚举）。doRegister 少于 4 元素被静默忽略（findAction 要求 size==4）。
// getComponent 不创建组件；doRegister 才是创建入口（getOrCreateComponent）。
//
// ### 坑 6：setShortcut 必须带 flags=NoAutoloading(0x4)
// flags=0 时 kglobalacceld 走 autoload 分支：动作非 fresh 则**忽略传入键**、
// 直接返回已存键（表现：gdbus 调用返回 [0] 且键没设上）。真改键必须
// NoAutoloading。另：busctl 调 setShortcut 报 "Too many parameters"（busctl
// 数组参数解析问题），用 gdbus 正常；Rust zbus 直接 call_method 正常。
//
// ### 坑 7：键编码 = Qt QKeySequence int（mods | keycode）
// Alt+Shift+A = 0x0A000041 = 167772225。修饰位 Shift=0x02000000 Ctrl=0x04000000
// Alt=0x08000000 Meta=0x10000000；键码字母=ASCII 大写(0x41..)、数字=0x30..、
// F1=0x01000030+。从 kglobalshortcutsrc 读回为字符串形式（如 "Alt+Shift+A"）。
//
// ### 坑 8：allShortcutInfos 的 D-Bus 签名是 a(ssssssaiai)
// = struct(6×string, ai, ai)（两个 int 数组字段），不是 a(ssssssaai)/a(ai)。
// Rust 侧反序列化写错字段数会 Signature mismatch（zbus 报
// "got a(ssssssaiai), expected a(ssssssaai)"），cleanup 静默失效。
//
// ### 坑 9：zbus5 API 与 zbus4 差异大
// - Proxy::receive_signal 的泛型不是信号参数类型（编译报 MemberName From<T>）
// - Connection::add_match_rule 不存在 → 用 zbus::fdo::DBusProxy::add_match_rule
// - MatchRule::builder() 的 sender/interface/member/path 返回 Result，需 ?/and_then
// - MessageStream::from(&conn) 不加规则收不到广播信号
//
// ### 未解决（交接给接手者）
// 组件注册/键设置/持久化/幂等全部验证通过，但**按快捷键不触发**（globalShortcutPressed
// 信号没到 atray 或没分发）。排查方向：
// 1. KWin 的 kglobalaccel 集成是否真的抓键并 emit（用 gdbus monitor 或 dbus-monitor
//    盯 /component/atray 的 globalShortcutPressed 信号,物理按键时观察）
// 2. Component 接口的 isActive 属性/需要调用 org.kde.kglobalaccel.Component 的
//    某些激活方法（KGlobalAccel 框架客户端在 kglobalacceld 侧有专用通道,
//    纯 D-Bus 第三方可能漏了 kglobalacceld 要求的状态机,如 GlobalShortcutsRegistry
//    的 grab 机制只在组件"present"时工作）
// 3. 备选:验证 portal 通道在**正常发行版**(非 Debian 打包缺陷)可用后,
//    以 portal 为主 + KGA 为 KDE 增强
// 4. 或检查 KWin 日志(org.kde.kglobalaccel debug)看按键是否被识别
//
// ## 设计（幂等语义）
//   - 固定组件 atray → kglobalshortcutsrc [atray] 段,系统设置里一个条目
//   - 启动同步宽松:动作已有键(用户/系统设置改过)→ 保留;无键 → 填默认
//   - IPC 改键 → 强制 setShortcut(NoAutoloading),即时生效
//   - 启动先清理历史 portal 残留(token_ashpd_* 带 atray 动作描述),一次性迁移

#![cfg(target_os = "linux")]

use ashpd::zbus;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

use crate::{TrayConfig, hide_main, load_effective_config, show_main};

/// KGA 固定组件名（系统设置里显示为 atray；动作持久化于 kglobalshortcutsrc [atray]）
pub const COMPONENT: &str = "atray";
const IFACE_DAEMON: &str = "org.kde.KGlobalAccel";
const IFACE_COMPONENT: &str = "org.kde.kglobalaccel.Component";
const DEST: &str = "org.kde.kglobalaccel";
const PATH_DAEMON: &str = "/kglobalaccel";

// Qt 修饰符位（QKeySequence int 编码：mods | keycode）
const MOD_SHIFT: i32 = 0x0200_0000;
const MOD_CTRL: i32 = 0x0400_0000;
const MOD_ALT: i32 = 0x0800_0000;
const MOD_META: i32 = 0x1000_0000;
const KEY_F1: i32 = 0x0100_0030;

/// org.kde.kglobalaccel 服务是否可用（KDE 桌面）。
pub fn available() -> bool {
    // 同步探测走 session bus 代价高；这里用环境近似 + 启动后异步确认。
    // 由 spawn 内部首次同步失败自动回退 portal（见 lib.rs 选择逻辑）。
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_ascii_lowercase().contains("kde"))
        .unwrap_or(false)
}

/// 启动 KGA 热键后端（async 任务：注册 + 信号监听 + 变更重同步）。
pub fn spawn(app: AppHandle) -> watch::Sender<bool> {
    let (tx, rx) = watch::channel(false);
    tauri::async_runtime::spawn(kga_loop(app.clone(), rx));
    tauri::async_runtime::spawn(signal_listener(app));
    tx
}

/// 主循环：启动同步（幂等）→ 等待变更通知 → 重同步。
async fn kga_loop(app: AppHandle, mut rx: watch::Receiver<bool>) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("atray: KGlobalAccel 连接失败: {e}");
            return;
        }
    };
    let mut first = true;
    loop {
        let cfg = load_effective_config();
        if first {
            // 一次性迁移:清理历史 portal 后端残留组件(token_ashpd_* 带 atray 动作)
            crate::portal::cleanup_stale_atray(&conn).await;
        }
        // 首轮宽松（动作已有键则保留——用户/系统设置改过的键不覆盖）；
        // 此后 notify（IPC 改键/增删插件）→ 强制以配置为准
        if let Err(e) = sync_all(&conn, &cfg, !first).await {
            eprintln!("atray: KGlobalAccel 同步失败: {e}");
        }
        first = false;
        let _ = rx.changed().await;
    }
}

/// 同步动作注册与按键。
/// `force` = true 表示用户显式改键（IPC），按键以配置为准覆盖；
/// false = 启动/配置变更后的宽松同步：动作已有键则保留（用户/系统设置改过）。
async fn sync_all(conn: &zbus::Connection, cfg: &TrayConfig, force: bool) -> zbus::Result<()> {
    // 1) 总快捷键 toggle
    let toggle_key = cfg.hotkey.as_deref().and_then(to_qt_keys);
    match (&cfg.hotkey, toggle_key) {
        (Some(_), Some(keys)) => {
            ensure_action(conn, "toggle", "呼出/隐藏 atray 覆盖层", &keys, force).await?;
        }
        _ => {
            // 未配置总快捷键：注销动作（若有）
            let _ = unregister_action(conn, "toggle").await;
        }
    }
    // 2) 插件快捷键 plugin:<id>
    let mut want: Vec<(String, String, Vec<i32>)> = Vec::new(); // (id, 描述, 键)
    for p in &cfg.plugins {
        if let Some(hk) = &p.hotkey {
            if let Some(keys) = to_qt_keys(hk) {
                let name = if p.name.is_empty() { p.id.clone() } else { p.name.clone() };
                want.push((p.id.clone(), format!("切换到 {name}"), keys));
            }
        }
    }
    let current = list_actions(conn).await?;
    // 注销已不存在的动作
    for (act, _desc) in &current {
        if act == "toggle" {
            continue;
        }
        if let Some(pid) = act.strip_prefix("plugin:") {
            if !want.iter().any(|(id, _, _)| id == pid) {
                let _ = unregister_action(conn, act).await;
            }
        }
    }
    for (id, desc, keys) in want {
        ensure_action(conn, &format!("plugin:{id}"), &desc, &keys, force).await?;
    }
    Ok(())
}

/// 确保动作存在且有键：注册（幂等）→ 宽松模式：无键才填；强制模式：覆盖。
async fn ensure_action(
    conn: &zbus::Connection,
    action: &str,
    desc: &str,
    keys: &[i32],
    force: bool,
) -> zbus::Result<()> {
    let action_id = action_id_of(action, desc);
    // doRegister：已存在则更新友好名，不新增（幂等核心）
    let _: () = call(conn, PATH_DAEMON, IFACE_DAEMON, "doRegister", &action_id).await?;
    const NO_AUTOLOADING: u32 = 0x4; // 真改键:否则 daemon 按已存设置 autoload,忽略新键
    if force {
        let _: Vec<i32> = call(
            conn,
            PATH_DAEMON,
            IFACE_DAEMON,
            "setShortcut",
            &(action_id, keys.to_vec(), NO_AUTOLOADING),
        )
        .await?;
        return Ok(());
    }
    // 宽松：读当前键，有则保留（用户/系统设置改过的键不覆盖）
    let cur: Vec<i32> = call(conn, PATH_DAEMON, IFACE_DAEMON, "shortcut", &action_id).await?;
    if cur.is_empty() {
        let _: Vec<i32> = call(
            conn,
            PATH_DAEMON,
            IFACE_DAEMON,
            "setShortcut",
            &(action_id, keys.to_vec(), NO_AUTOLOADING),
        )
        .await?;
    }
    Ok(())
}

async fn unregister_action(conn: &zbus::Connection, action: &str) -> zbus::Result<()> {
    let _: bool = call(
        conn,
        PATH_DAEMON,
        IFACE_DAEMON,
        "unregister",
        &(COMPONENT.to_string(), action.to_string()),
    )
    .await?;
    Ok(())
}

/// 当前组件动作名列表（不含已清空键的残留? shortcutNames 全量）。
async fn list_actions(conn: &zbus::Connection) -> zbus::Result<Vec<(String, String)>> {
    let names: Vec<String> = call(
        conn,
        &format!("/component/{COMPONENT}"),
        IFACE_COMPONENT,
        "shortcutNames",
        &(),
    )
    .await?;
    Ok(names.into_iter().map(|n| (n, String::new())).collect())
}

/// 4 元素 actionId：[组件, 动作, 组件友好名, 动作友好名]
fn action_id_of(action: &str, desc: &str) -> Vec<String> {
    vec![
        COMPONENT.to_string(),
        action.to_string(),
        COMPONENT.to_string(),
        desc.to_string(),
    ]
}

async fn call<A, R>(conn: &zbus::Connection, path: &str, iface: &str, method: &str, args: &A) -> zbus::Result<R>
where
    A: serde::ser::Serialize + zbus::zvariant::Type,
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    let msg = conn
        .call_method(Some(DEST), path, Some(iface), method, args)
        .await?;
    msg.body().deserialize().map_err(Into::into)
}

/// 监听组件激活信号（按下快捷键）并分发。
///
/// ⚠ 未解决（见文件头「未解决」节）：订阅已注册（fdo AddMatch），但实测按键
/// 不触发 dispatch。接手者从这里开始排查。
async fn signal_listener(app: AppHandle) {
    use futures_util::StreamExt;
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("atray: KGlobalAccel 信号连接失败: {e}");
            return;
        }
    };
    // 订阅组件激活信号（zbus5：经 org.freedesktop.DBus AddMatch 注册规则）
    let rule = match zbus::MatchRule::builder()
        .sender(DEST)
        .and_then(|b| b.path(format!("/component/{COMPONENT}")))
        .and_then(|b| b.interface(IFACE_COMPONENT))
        .and_then(|b| b.member("globalShortcutPressed"))
        .map(|b| b.msg_type(zbus::message::Type::Signal).build())
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("atray: 匹配规则构造失败: {e}");
            return;
        }
    };
    let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("atray: DBus 代理失败: {e}");
            return;
        }
    };
    if let Err(e) = dbus_proxy.add_match_rule(rule).await {
        eprintln!("atray: globalShortcutPressed 订阅失败: {e}");
        return;
    }
    let mut stream = zbus::MessageStream::from(&conn);
    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        let Ok((comp, action, _ts)) = msg.body().deserialize::<(String, String, i64)>() else {
            continue;
        };
        if comp == COMPONENT {
            dispatch(&app, &action);
        }
    }
}

/// 快捷键分发（与 native/portal 回调语义一致）。
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

/// atray 快捷键串（"Alt+Shift+A"）→ QKeySequence int 编码（mods | keycode）。
pub(crate) fn to_qt_keys(hk: &str) -> Option<Vec<i32>> {
    let mut parts: Vec<&str> = hk.split('+').collect();
    if parts.is_empty() {
        return None;
    }
    let key = parts.pop()?;
    let mut mods = 0i32;
    for m in parts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "alt" => MOD_ALT,
            "ctrl" | "control" => MOD_CTRL,
            "shift" => MOD_SHIFT,
            "super" | "meta" | "logo" | "cmd" | "win" => MOD_META,
            _ => return None,
        };
    }
    if mods == 0 {
        return None; // 无修饰键的全局热键不自动设置
    }
    let kc = if key.len() == 1 {
        let c = key.chars().next()?;
        if c.is_ascii_alphabetic() {
            c.to_ascii_uppercase() as i32 // Qt::Key_A = 0x41 = 'A'
        } else if c.is_ascii_digit() {
            c as i32 // Qt::Key_0 = 0x30 = '0'
        } else {
            return None;
        }
    } else if key.len() >= 2 && key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit())
    {
        let n: i32 = key[1..].parse().ok()?;
        if !(1..=24).contains(&n) {
            return None;
        }
        KEY_F1 + (n - 1) // Qt::Key_F1 = 0x01000030
    } else {
        match key {
            "Return" | "Enter" => 0x0100_0004,
            "Escape" | "Esc" => 0x0100_0000,
            "Tab" => 0x0100_0001,
            "Backspace" => 0x0100_0003,
            "Delete" => 0x0100_0007,
            "Insert" => 0x0100_0006,
            "Home" => 0x0100_0010,
            "End" => 0x0100_0011,
            "PageUp" => 0x0100_0016,
            "PageDown" => 0x0100_0017,
            "Up" => 0x0100_0013,
            "Down" => 0x0100_0015,
            "Left" => 0x0100_0012,
            "Right" => 0x0100_0014,
            _ => return None,
        }
    };
    Some(vec![mods | kc])
}
