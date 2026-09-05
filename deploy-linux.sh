#!/usr/bin/env bash
# 部署 atray Linux 版到 101（Debian/KDE，Wayland 会话）：传二进制 + renderer → 运行。
# 用法：bash ./deploy-linux.sh
#
# 前提：
#   1) 101 已装运行时依赖：
#      sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libayatana-appindicator3-1
#   2) 构建：cd src-tauri && cargo build --release
#   3) 用户在 101 图形会话已登录（进程以该会话的 Wayland/DBus 环境启动）
#
# 热键：Wayland 会话走 XDG Desktop Portal GlobalShortcuts（KDE/GNOME 通用）——
# 首次启动会弹系统对话框确认快捷键；X11 会话自动回退 native 注册。
set -u
HOST=tony@192.168.0.101
BIN=./src-tauri/target/release/atray
RENDERER=./renderer
DEST=/home/tony/atray

if [ ! -f "$BIN" ]; then
  echo "未找到 $BIN，先构建（cd src-tauri && cargo build --release）"; exit 1
fi

scp -q "$BIN" "$HOST:/tmp/atray" || { echo scp 二进制失败; exit 1; }
timeout 30 ssh "$HOST" "rm -rf /tmp/atray-renderer"
scp -qr "$RENDERER" "$HOST:/tmp/atray-renderer" || { echo scp renderer 失败; exit 1; }

timeout 60 ssh "$HOST" "
  pkill -x atray 2>/dev/null
  sleep 1
  mkdir -p $DEST
  cp /tmp/atray $DEST/atray
  rm -rf $DEST/renderer
  cp -r /tmp/atray-renderer $DEST/renderer
  chmod +x $DEST/atray
  # 在用户的图形会话里启动（继承其 Wayland/DBus/X11 环境）
  UID_N=\$(id -u)
  export XDG_RUNTIME_DIR=/run/user/\$UID_N
  export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/\$UID_N/bus
  if [ -e /run/user/\$UID_N/wayland-0 ]; then
    export WAYLAND_DISPLAY=wayland-0
  fi
  # X11(GDK)与桌面类型环境:从图形会话进程继承(atray 据此选热键后端)
  for p in plasmashell kwin_wayland gnome-shell; do
    pid=\$(pgrep -x \$p 2>/dev/null | head -1)
    if [ -n \"\$pid\" ]; then
      eval \"\$(tr '\0' '\n' < /proc/\$pid/environ 2>/dev/null | grep -E '^(DISPLAY|XAUTHORITY|XDG_CURRENT_DESKTOP)=')\"
      export DISPLAY XAUTHORITY XDG_CURRENT_DESKTOP
      [ -n \"\${DISPLAY:-}\" ] && break
    fi
  done
  setsid nohup $DEST/atray > /tmp/atray-run.log 2>&1 < /dev/null & disown
  sleep 6
  pgrep -x atray > /dev/null && echo 'atray 运行中' || { echo '启动失败，日志:'; tail -8 /tmp/atray-run.log; }
  echo '--- 运行日志 ---'; tail -8 /tmp/atray-run.log
" 2>&1
echo "部署完成"
