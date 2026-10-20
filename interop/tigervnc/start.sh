set -e
if [ -n "$VNC_PASSWORD" ]; then
  printf '%s\n' "$VNC_PASSWORD" | vncpasswd -f > /tmp/passwd
  set -- -SecurityTypes VncAuth -PasswordFile /tmp/passwd
else
  set -- -SecurityTypes None
fi
exec Xvnc :0 -rfbport 5900 -geometry 1024x768 -depth 24 -desktop justvnc-test -AlwaysShared "$@"
