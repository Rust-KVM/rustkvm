#!/bin/sh

if [ -f "/userdata/rustkvm/devmode.enable" ]; then
	mkdir -p /userdata/dropbear/etc
	mkdir -p /userdata/dropbear/.ssh
	ln -sf /userdata/dropbear/etc /etc/dropbear
	ln -sf /userdata/dropbear/.ssh /root/.ssh
	# Called on every app start: leave a running daemon alone instead of
	# failing with "Address already in use".
	if ! pidof dropbear >/dev/null 2>&1; then
		dropbear -R -E -p 0.0.0.0:22 -P /run/dropbear.pid -s
	fi
else
	# Kill Dropbear if it's already running
	killall dropbear >/dev/null 2>&1 || true
fi
