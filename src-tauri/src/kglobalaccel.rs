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
// ⚠ 但只带 0x4 会漏 SetPresent（坑 10，按键不触发）——设键一律用 0x6。
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
// ### 坑 10：setShortcut 的 flags 必须带 SetPresent(0x2)！——2026-09 按键不触发根因
// 只传 NoAutoloading(0x4) 时键能存进配置，但动作不被标记 present；
// GlobalShortcut::setActive() 抓键前置条件是 _isPresent，不 present → 键不进
// registry 的 _active_keys → 按键永远匹配不到 → globalShortcutPressed 不发。
// 表现：注册/设键/持久化全通、订阅也在，就是按了没反应；Component::isActive()=false
// 是可靠诊断信号。真实 KDE 框架客户端都带 SetPresent，第三方直连最容易漏。
// 修法：flags = SetPresent | NoAutoloading = 0x6（FLAG_SET_KEY）。
//
// ### 未解决（历史交接记录，已定位根因见坑 10）
// 按键不触发问题的排查过程：见 git 交接存档 15cc67c 与 STATUS-linux-wayland.md；
// dbus-monitor 验证、kglobalacceld 源码（globalshortcut.cpp setActive /
// kglobalacceld.cpp setShortcutKeys）定位到 present 旗标缺失，0x6 修复。
//
// ## 设计 v2（desired-state 对账，2026-09 决策：应用配置为唯一改键入口）
//   - 固定组件 atray → kglobalshortcutsrc [atray] 段，系统设置里一个条目
//   - config.json = 期望态。启动/每次 IPC 变更/周期性 watchdog 都跑同一个
//     reconcile_all()：读系统实际 → 与期望 diff → 最小写操作收敛。
//     相同 → 零 D-Bus 写；不同 → 改回配置值；多余动作 → 注销。
//   - 无「宽松/强制」两态：系统侧任何偏离（用户去系统设置改键等）都会在对账时
//     被还原。理由：KDE 的 kglobalshortcutsrc 只是全局热键的强制承载，产品上不
//     鼓励用户在系统设置里改 atray 动作——改键请用 atray 设置页（两边唯一入口）。
//   - 启动先清理历史 portal 残留(token_ashpd_* 带 atray 动作描述)，一次性迁移

#![cfg(target_os = "linux")]

use ashpd::zbus;
use std::path::PathBuf;
use std::time::Duration;
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

// kglobalacceld setShortcut flags（KGlobalAccelD::SetShortcutFlag，见 daemon 头文件）
const FLAG_SET_PRESENT: u32 = 0x2;    // 标记动作「在场」——抓键的前提！
const FLAG_NO_AUTOLOADING: u32 = 0x4; // 真改键：否则 daemon 按已存设置 autoload，忽略新键
/// 设键必须同时带两个旗标：仅 NoAutoloading 能存键但动作不 present → 键不被抓取
/// → 按键无信号（Component::isActive()=false，本 bug 即 2026-09 卡点根因）。
const FLAG_SET_KEY: u32 = FLAG_SET_PRESENT | FLAG_NO_AUTOLOADING;

/// org.kde.kglobalaccel 服务是否可用（KDE 桌面）。
pub fn available() -> bool {
    // 同步探测走 session bus 代价高；这里用环境近似 + 启动后异步确认。
    // 由 spawn 内部首次同步失败自动回退 portal（见 lib.rs 选择逻辑）。
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_ascii_lowercase().contains("kde"))
        .unwrap_or(false)
}

/// 启动 KGA 热键后端（async 任务：对账循环 + 信号监听 + 漂移 watchdog）。
pub fn spawn(app: AppHandle) -> watch::Sender<bool> {
    let (tx, rx) = watch::channel(false);
    tauri::async_runtime::spawn(kga_loop(rx));
    tauri::async_runtime::spawn(signal_listener(app));
    tauri::async_runtime::spawn(watchdog());
    tx
}

/// 主循环：启动（清理历史 portal 残留 → 首次对账）→ 等待变更通知 → 重新对账。
///
/// 每个触发点走同一个 reconcile_all()，无「首次宽松/后续强制」两态：
/// 对账幂等（实际已 == 期望时零 D-Bus 写），多触发几次也只是空转。
async fn kga_loop(mut rx: watch::Receiver<bool>) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("atray: KGlobalAccel 连接失败: {e}");
            return;
        }
    };
    // 一次性迁移:清理历史 portal 后端残留组件(token_ashpd_* 带 atray 动作)
    crate::portal::cleanup_stale_atray(&conn).await;
    loop {
        if let Err(e) = reconcile_all(&conn, &load_effective_config()).await {
            eprintln!("atray: KGlobalAccel 对账失败: {e}");
        }
        let _ = rx.changed().await;
    }
}

/// 对账：系统实际状态收敛到配置（期望态 = config.json，应用内设置为唯一改键入口）。
///
/// 1) 期望动作：缺则 doRegister（幂等，不重复建组件）；随后**无条件**
///    setShortcut(keys, SetPresent|NoAutoloading)——present 状态没有读接口，
///    且历史 0x4 注册的动作 present=false，只能靠每次设键顺带修复。
///    写操作仅发生在：启动、IPC 变更、外部改键（watchdog 触发）——低频可接受。
/// 2) 多余动作（配置里已删除/无热键）→ unregister。
async fn reconcile_all(conn: &zbus::Connection, cfg: &TrayConfig) -> zbus::Result<()> {
    let want = desired_actions(cfg);
    // 实际已注册的动作名（shortcutNames 全量）
    let names: Vec<String> = call(
        conn,
        &format!("/component/{COMPONENT}"),
        IFACE_COMPONENT,
        "shortcutNames",
        &(),
    )
    .await?;
    // 1) 注册并对齐键（含 present 标记——抓键前提）
    for (act, desc, keys) in &want {
        let action_id = action_id_of(act, desc);
        // doRegister：已存在则更新友好名，不新增（幂等核心）
        let _: () = call(conn, PATH_DAEMON, IFACE_DAEMON, "doRegister", &action_id).await?;
        let _: Vec<i32> = call(
            conn,
            PATH_DAEMON,
            IFACE_DAEMON,
            "setShortcut",
            &(action_id, keys.clone(), FLAG_SET_KEY),
        )
        .await?;
    }
    // 2) 注销多余动作
    for act in &names {
        if want.iter().any(|(a, _, _)| a == act) {
            continue;
        }
        unregister_action(conn, act).await?;
    }
    Ok(())
}

/// 期望动作表：(动作名, 描述, Qt 键编码)。配置里没有/无法表达的快捷键不产生期望。
fn desired_actions(cfg: &TrayConfig) -> Vec<(String, String, Vec<i32>)> {
    let mut want: Vec<(String, String, Vec<i32>)> = Vec::new();
    if let Some(hk) = cfg.hotkey.as_deref().and_then(to_qt_keys) {
        want.push(("toggle".into(), "呼出/隐藏 atray 覆盖层".into(), hk));
    }
    for p in &cfg.webapps {
        if let Some(hk) = p.hotkey.as_deref().and_then(to_qt_keys) {
            let name = if p.name.is_empty() { p.id.clone() } else { p.name.clone() };
            want.push((format!("webapp:{}", p.id), format!("切换到 {name}"), hk));
        }
    }
    want
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

/// 漂移 watchdog：周期读 kglobalshortcutsrc，内容变化（用户系统设置改键等）
/// → 重新对账还原。**内容比较**而非 mtime：本端 setShortcut 也会重写该文件，
/// 但写出的内容与期望一致——内容相同即跳过，天然免疫自触发循环与 daemon
/// 写回延迟（KConfig 经临时文件原子替换，读到的一定是完整新/旧内容）。
async fn watchdog() {
    let path = match rc_path() {
        Some(p) => p,
        None => {
            eprintln!("atray: 找不到 kglobalshortcutsrc 路径，watchdog 停用");
            return;
        }
    };
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("atray: watchdog D-Bus 连接失败: {e}");
            return;
        }
    };
    let mut seen: Option<String> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let content = std::fs::read_to_string(&path).ok();
        if content == seen {
            continue;
        }
        if let Err(e) = reconcile_all(&conn, &load_effective_config()).await {
            eprintln!("atray: watchdog 对账失败: {e}");
        }
        seen = std::fs::read_to_string(&path).ok();
    }
}

fn rc_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("kglobalshortcutsrc"))
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
    } else if let Some(pid) = id.strip_prefix("webapp:") {
        show_main(&win);
        let _ = app.emit("atray-webapp-activate", pid.to_string());
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
