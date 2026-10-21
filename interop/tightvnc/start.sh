set -e
exec Xtightvnc :0 -geometry "${GEOMETRY:-800x600}" -depth 24 -rfbport 5900 -alwaysshared -desktop justvnc-tightvnc
