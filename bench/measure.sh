#!/bin/bash
# measure.sh: RAM and CPU of every process whose name matches a pattern.
#
#   bench/measure.sh '^(soloist|soloist-mpris)$' 60
#   bench/measure.sh '^spotify$' 60
#
# Run it while music plays. RAM is PSS (shared pages split between the
# processes that share them, so the total is not inflated), sampled every
# INTERVAL seconds and averaged. CPU is the share of one core used over the
# real elapsed time. The process set is read again at every sample, so
# helpers that start or exit during the window are counted while they live.
set -euo pipefail

INTERVAL=5

pattern=${1:?usage: measure.sh <process-name-regex> [seconds]}
seconds=${2:-60}
[[ $seconds =~ ^[1-9][0-9]*$ ]] || { echo "measure.sh: seconds must be a positive whole number" >&2; exit 2; }
hz=$(getconf CLK_TCK)

pids() { pgrep -- "$pattern" || true; }

# User + system CPU ticks of one process; empty when it is gone.
ticks_of() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || true; }

# Total PSS of the matching processes, in kB.
pss() {
  local total=0 pid kb
  for pid in $(pids); do
    kb=$(awk '/^Pss:/ {print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null || true)
    total=$((total + ${kb:-0}))
  done
  echo "$total"
}

count=$(pids | wc -l)
(( count > 0 )) || { echo "measure.sh: no process matches $pattern" >&2; exit 1; }

# CPU is summed per process: ticks a process used since the window opened,
# or since it started when it appeared later. One that exits mid-window
# keeps what it used up to its last sample.
declare -A first last
for pid in $(pids); do first[$pid]=$(ticks_of "$pid"); done

started=$(date +%s%N)
samples=0 pss_sum=0 pss_peak=0
while :; do
  sample=$(pss)
  pss_sum=$((pss_sum + sample))
  (( sample > pss_peak )) && pss_peak=$sample
  samples=$((samples + 1))
  for pid in $(pids); do
    now=$(ticks_of "$pid")
    [[ -n $now ]] || continue
    [[ -n ${first[$pid]:-} ]] || first[$pid]=0
    last[$pid]=$now
  done
  elapsed_ms=$(( ($(date +%s%N) - started) / 1000000 ))
  remaining_ms=$(( seconds * 1000 - elapsed_ms ))
  (( remaining_ms > 0 )) || break
  sleep "$(awk -v r="$remaining_ms" -v i="$INTERVAL" 'BEGIN { s = r / 1000; print (s < i ? s : i) }')"
done
elapsed_ms=$(( ($(date +%s%N) - started) / 1000000 ))

used=0
for pid in "${!last[@]}"; do used=$((used + last[$pid] - first[$pid])); done

awk -v n="$count" -v avg=$((pss_sum / samples)) -v peak="$pss_peak" \
    -v t="$used" -v hz="$hz" -v ms="$elapsed_ms" 'BEGIN {
  printf "processes: %d\n", n
  printf "RAM (PSS): %.0f MB average, %.0f MB peak\n", avg / 1024, peak / 1024
  printf "CPU: %.1f%% of one core over %.1fs\n", t / hz / (ms / 1000) * 100, ms / 1000
}'
