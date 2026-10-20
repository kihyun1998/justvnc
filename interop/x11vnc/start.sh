set -e
Xvfb :0 -screen 0 800x600x24 &
sleep 1
if [ -n "$VNC_PASSWORD" ]; then
  set -- -passwd "$VNC_PASSWORD"
else
  set -- -nopw
fi
exec x11vnc -display :0 -rfbport 5900 -rfbversion "${RFB_VERSION:-3.8}" -forever -shared -desktop justvnc-x11vnc "$@"
