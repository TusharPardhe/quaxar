#!/bin/bash
# Host and quaxar resource samples every INTERVAL seconds into a CSV.
# Usage: sample.sh <out.csv> <interval_s> <duration_s>
# Columns: unix, host_cpu_busy_pct, load1, mem_used_kb, quaxar_rss_kb,
#          quaxar_cpu_ticks, quaxar_read_bytes, quaxar_write_bytes, store_bytes
set -u
OUT=${1:?out.csv}
INTERVAL=${2:-10}
DURATION=${3:-3600}

cpu() { awk '/^cpu /{idle=$5+$6; t=0; for(i=2;i<=NF;i++) t+=$i; print t, idle}' /proc/stat; }

echo "unix,host_cpu_busy_pct,load1,mem_used_kb,quaxar_rss_kb,quaxar_cpu_ticks,quaxar_read_bytes,quaxar_write_bytes,store_bytes" >"$OUT"
end=$(( $(date +%s) + DURATION ))
read -r t0 i0 < <(cpu)
while [ "$(date +%s)" -lt "$end" ]; do
  sleep "$INTERVAL"
  read -r t1 i1 < <(cpu)
  busy=$(awk -v t="$((t1 - t0))" -v i="$((i1 - i0))" 'BEGIN{ if (t>0) printf "%.1f", 100*(t-i)/t; else print 0 }')
  t0=$t1; i0=$i1
  load=$(cut -d' ' -f1 /proc/loadavg)
  mem=$(awk '/MemTotal/{t=$2} /MemAvailable/{a=$2} END{print t-a}' /proc/meminfo)
  pid=$(pgrep -x quaxar | head -1 || true)
  rss=""; ticks=""; rb=""; wb=""; store=""
  if [ -n "$pid" ]; then
    rss=$(awk '/^VmRSS/{print $2}' /proc/"$pid"/status)
    ticks=$(awk '{print $14+$15}' /proc/"$pid"/stat)
    rb=$(awk '/^read_bytes/{print $2}' /proc/"$pid"/io)
    wb=$(awk '/^write_bytes/{print $2}' /proc/"$pid"/io)
    store=$(du -sb /data/db 2>/dev/null | cut -f1)
  fi
  echo "$(date +%s),$busy,$load,$mem,$rss,$ticks,$rb,$wb,$store" >>"$OUT"
done
