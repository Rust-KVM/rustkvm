#!/bin/sh

if [ -f "/userdata/rustkvm/devmode.enable" ]; then
	mkdir -p /userdata/dropbear/etc
	mkdir -p /userdata/dropbear/.ssh
	ln -sf /userdata/dropbear/etc /etc/dropbear
	ln -sf /userdata/dropbear/.ssh /root/.ssh
	#TODO: setup syslog instead of -E(log to stderr)
	dropbear -R -E -p 0.0.0.0:22 -P /run/dropbear.pid -s
	# dropbear -R -E -p 0.0.0.0:22 -P /run/dropbear.pid
else
	# Kill Dropbear if it's already running
	killall dropbear >/dev/null 2>&1 || true
fi
