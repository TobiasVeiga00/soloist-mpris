#!/bin/bash
# measure.sh: RAM and CPU of every process whose name matches a pattern.
#
#   bench/measure.sh '^(soloist|soloist-mpris)$' 60
#   bench/measure.sh '^spotify$' 60
#
# Run it while music plays. RAM is PSS (shared pages split between the
# processes that share them, so the total is not inflated), sampled every
# 5 seconds and averaged; CPU is the share of one core used over the window.
set -euo pipefail

pattern=${1:?usage: measure.sh <process-name-regex> [seconds]}
seconds=${2:-60}
hz=$(getconf CLK_TCK)

pids() { pgrep -d ' ' -- "$pattern" || true; }

# Total CPU ticks (user + system) of the matching processes.
ticks() {
  local total=0 pid
  for pid in $(pids); do
    [[ -r /proc/$pid/stat ]] || continue
    total=$((total + $(awk '{print $14 + $15}' "/proc/$pid/stat" 2>/dev/null || echo 0)))
  done
  echo "$total"
}

# Total PSS of the matching processes, in kB.
pss() {
  local total=0 pid
  for pid in $(pids); do
    total=$((total + $(awk '/^Pss:/ {print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null || echo 0)))
  done
  echo "$total"
}

count=$(wc -w <<<"$(pids)")
(( count > 0 )) || { echo "measure.sh: no process matches $pattern" >&2; exit 1; }

start=$(ticks)
samples=0 pss_sum=0 pss_peak=0
for ((elapsed = 0; elapsed < seconds; elapsed += 5)); do
  sample=$(pss)
  pss_sum=$((pss_sum + sample))
  (( sample > pss_peak )) && pss_peak=$sample
  samples=$((samples + 1))
  sleep 5
done
end=$(ticks)

awk -v n="$count" -v avg=$((pss_sum / samples)) -v peak="$pss_peak" \
    -v t=$((end - start)) -v hz="$hz" -v s="$seconds" 'BEGIN {
  printf "processes: %d\n", n
  printf "RAM (PSS): %.0f MB average, %.0f MB peak\n", avg / 1024, peak / 1024
  printf "CPU: %.1f%% of one core over %ds\n", t / hz / s * 100, s
}'
