#!/usr/bin/env bash
# 修复 Debian 13 (trixie) KDE 的 GlobalShortcuts portal 对话框缺失问题。
#
# 背景（2026-09 实测）：
#   - Debian trixie 的 xdg-desktop-portal-kde 6.3.5 二进制未内嵌/未安装对话框 QML
#     （仅 qrc:/UserInfoDialog.qml），导致 portal GlobalShortcuts 的绑定确认框
#     无法弹出（QuickDialog rootObjects 为空 → 静默失败，请求永久挂起）。
#     上游 v6.3.5 发布包本身缺失这批 QML（GlobalShortcutsDialog 等为 6.4+ 文件），
#     Debian 忠实继承了该缺陷；sid 6.7.4 起已内嵌完整。
#   - 另缺 org.kde.ki18n 的 QML 模块声明（qmldir），GlobalShortcutsDialog
#     import org.kde.ki18n 需要它。
#
# 修复方式（幂等，可重复执行）：
#   1. 补 org.kde.ki18n 模块：qmldir + 符号链接指向已安装的 libKF6I18nQml.so
#   2. 从上游 master 安装 portal 对话框 QML 到 org.kde.xdgdesktopportal 模块目录
#
# 上游已修复（6.7+）后可删除本脚本。
set -eu

QMLDIR_QT6=/usr/lib/x86_64-linux-gnu/qt6/qml
KI18N_DIR=$QMLDIR_QT6/org/kde/ki18n
PORTAL_DIR=$QMLDIR_QT6/org/kde/xdgdesktopportal

# ---- 1) org.kde.ki18n 模块 ----
if [ ! -f "$KI18N_DIR/qmldir" ]; then
  mkdir -p "$KI18N_DIR"
  ln -sf /usr/lib/x86_64-linux-gnu/libKF6I18nQml.so.6 "$KI18N_DIR/libKF6I18nQml.so"
  printf 'module org.kde.ki18n\nplugin KF6I18nQml\n' > "$KI18N_DIR/qmldir"
  echo "已补 org.kde.ki18n 模块 ($KI18N_DIR)"
else
  echo "org.kde.ki18n 模块已存在，跳过"
fi

# ---- 2) portal 对话框 QML（org.kde.xdgdesktopportal）----
if [ ! -f "$PORTAL_DIR/qmldir" ]; then
  mkdir -p "$PORTAL_DIR/region-select"
  BASE=https://invent.kde.org/plasma/xdg-desktop-portal-kde/-/raw/master/src
  FILES="AccessDialog.qml AppChooserDialog.qml DynamicLauncherDialog.qml
         GlobalShortcutsDialog.qml InputCaptureDialog.qml PipeWireDelegate.qml
         PipeWireLayout.qml PortalDialog.qml RemoteDesktopDialog.qml
         ScreenChooserDialog.qml ScreenChooserDialogTemplate.qml ScreenshotDialog.qml
         UsbDialog.qml WallpaperDialog.qml"
  (cd "$PORTAL_DIR" && for f in $FILES; do
    curl -fsSL "$BASE/$f" -o "$f" || echo "警告: 下载 $f 失败"
  done
  for f in RegionSelectOverlay.qml FloatingTextBox.qml FloatingBackground.qml; do
    curl -fsSL "$BASE/region-select/$f" -o "region-select/$f" || echo "警告: 下载 region-select/$f 失败"
  done)
  cat > "$PORTAL_DIR/qmldir" <<'QM'
module org.kde.xdgdesktopportal
AccessDialog 1.0 AccessDialog.qml
AppChooserDialog 1.0 AppChooserDialog.qml
DynamicLauncherDialog 1.0 DynamicLauncherDialog.qml
GlobalShortcutsDialog 1.0 GlobalShortcutsDialog.qml
InputCaptureDialog 1.0 InputCaptureDialog.qml
PipeWireDelegate 1.0 PipeWireDelegate.qml
PipeWireLayout 1.0 PipeWireLayout.qml
PortalDialog 1.0 PortalDialog.qml
RemoteDesktopDialog 1.0 RemoteDesktopDialog.qml
ScreenChooserDialog 1.0 ScreenChooserDialog.qml
ScreenChooserDialogTemplate 1.0 ScreenChooserDialogTemplate.qml
ScreenshotDialog 1.0 ScreenshotDialog.qml
UsbDialog 1.0 UsbDialog.qml
WallpaperDialog 1.0 WallpaperDialog.qml
QM
  chmod -R a+rX "$PORTAL_DIR"
  echo "已装 portal 对话框 QML ($PORTAL_DIR)"
else
  echo "portal 对话框 QML 已存在，跳过"
fi

echo "修复完成。重启相关服务: systemctl --user restart plasma-xdg-desktop-portal-kde"
