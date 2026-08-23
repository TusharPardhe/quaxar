#!/bin/sh
set -eu

if [ "$(id -u)" -eq 0 ]; then
    if [ ! -e /var/lib/quaxar/.quaxar-owned ]; then
        chown -R quaxar:quaxar /var/lib/quaxar
        touch /var/lib/quaxar/.quaxar-owned
        chown quaxar:quaxar /var/lib/quaxar/.quaxar-owned
    fi
    exec gosu quaxar "$@"
fi

exec "$@"
