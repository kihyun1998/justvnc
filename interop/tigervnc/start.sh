set -e
if [ -n "$VNC_PASSWORD" ]; then
  printf '%s\n' "$VNC_PASSWORD" | vncpasswd -f > /tmp/passwd
  set -- -SecurityTypes VncAuth -PasswordFile /tmp/passwd
else
  set -- -SecurityTypes None
fi
# GEOMETRY: screen size (default 1024x768). XVNC_ARGS: extra Xvnc arguments, e.g. -retro.
# shellcheck disable=SC2086
exec Xvnc :0 -rfbport 5900 -geometry "${GEOMETRY:-1024x768}" -depth 24 -desktop justvnc-test -AlwaysShared "$@" $XVNC_ARGS
