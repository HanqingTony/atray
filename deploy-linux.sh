#!/usr/bin/env bash
# 部署 atray Linux 版到 101（Debian/XFCE）：传二进制 + renderer → 运行。
# 用法：bash ./deploy-linux.sh
#
# 前提：101 已装运行时依赖（Debian）：
#   sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libayatana-appindicator3-1
set -u
HOST=tony@192.168.0.101
BIN=./src-tauri/target/release/atray
RENDERER=./renderer
DEST=/home/tony/atray

if [ ! -f "$BIN" ]; then
  echo "未找到 $BIN，先构建（cargo build --release）"; exit 1
fi

scp -q "$BIN" "$HOST:/tmp/atray" || { echo scp 二进制失败; exit 1; }
timeout 30 ssh "$HOST" "rm -rf /tmp/atray-renderer"
scp -qr "$RENDERER" "$HOST:/tmp/atray-renderer" || { echo scp renderer 失败; exit 1; }

timeout 60 ssh "$HOST" "
  pkill -f 'atray' 2>/dev/null
  sleep 1
  mkdir -p $DEST
  cp /tmp/atray $DEST/atray
  rm -rf $DEST/renderer
  cp -r /tmp/atray-renderer $DEST/renderer
  chmod +x $DEST/atray
  # 在用户的 X11 会话里启动（XFCE 单用户桌面通常 :0）
  DISPLAY=:0 setsid nohup $DEST/atray > /tmp/atray-run.log 2>&1 < /dev/null & disown
  sleep 5
  pgrep -f 'atray' > /dev/null && echo 'atray 运行中' || { echo '启动失败，日志:'; tail -5 /tmp/atray-run.log; }
  echo '--- panic.log ---'; cat $DEST/panic.log 2>/dev/null || echo '(无 panic)'
" 2>&1
echo "部署完成"
