#/bin/bash

# start xorg
Xorg -noreset -logfile /dev/null -config ./xorg.conf  &

# start dbus
mkdir -p /var/run/dbus
dbus-daemon --config-file=/usr/share/dbus-1/system.conf  --print-address &

sleep 2

# run fuzzer
cargo run --release

