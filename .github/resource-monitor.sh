#!/bin/bash

# Samples memory / disk / CPU while a job runs, then reports the peaks against
# what the runner actually provides.
#
# Purpose: several workflows provision g6-standard-16 (16 vCPU / 64 GB) VMs.
# Nothing verified those sizes were needed. This makes real usage visible in
# the job summary so plan sizes can be checked against evidence.
#
#   resource-monitor.sh start  [label]   # begin sampling in the background
#   resource-monitor.sh report [label]   # stop sampling, print + summarise peaks
#
# Never fails a job: every path exits 0.

LABEL="${2:-job}"
STATE_DIR="${RESOURCE_MONITOR_DIR:-/var/tmp/resource-monitor}"
SAMPLES="${STATE_DIR}/${LABEL}.tsv"
PIDFILE="${STATE_DIR}/${LABEL}.pid"
INTERVAL="${RESOURCE_MONITOR_INTERVAL:-10}"
# Watch the filesystem holding the workspace; that is what fills up.
DISK_PATH="${RESOURCE_MONITOR_DISK_PATH:-${GITHUB_WORKSPACE:-$PWD}}"

mem_total_mb() { awk '/^MemTotal:/ {print int($2/1024)}' /proc/meminfo; }

sample_once() {
    local total avail used swap_total swap_free swap_used disk load
    total=$(awk '/^MemTotal:/ {print int($2/1024)}' /proc/meminfo)
    avail=$(awk '/^MemAvailable:/ {print int($2/1024)}' /proc/meminfo)
    # MemTotal-MemAvailable tracks memory the workload actually needs, unlike
    # "used", which counts reclaimable page cache against you.
    used=$(( total - avail ))
    swap_total=$(awk '/^SwapTotal:/ {print int($2/1024)}' /proc/meminfo)
    swap_free=$(awk '/^SwapFree:/ {print int($2/1024)}' /proc/meminfo)
    swap_used=$(( swap_total - swap_free ))
    disk=$(df -m --output=used "$DISK_PATH" 2>/dev/null | tail -1 | tr -d ' ')
    load=$(awk '{print $1}' /proc/loadavg)
    printf '%s\t%s\t%s\t%s\t%s\n' \
        "$(date -u +%s)" "$used" "$swap_used" "${disk:-0}" "$load"
}

case "${1:-}" in
start)
    mkdir -p "$STATE_DIR" || exit 0
    printf 'epoch\tmem_used_mb\tswap_used_mb\tdisk_used_mb\tload1\n' > "$SAMPLES"
    (
        while true; do
            sample_once >> "$SAMPLES"
            sleep "$INTERVAL"
        done
    ) &
    echo $! > "$PIDFILE"
    echo "INFO: resource-monitor started for '${LABEL}' (pid $(cat "$PIDFILE"), every ${INTERVAL}s)"
    ;;

report)
    if [ -f "$PIDFILE" ]; then
        kill "$(cat "$PIDFILE")" 2>/dev/null
        rm -f "$PIDFILE"
    fi
    if [ ! -s "$SAMPLES" ]; then
        echo "WARNING: no samples collected for '${LABEL}', nothing to report"
        exit 0
    fi
    # One final sample so short jobs always have at least one data point.
    sample_once >> "$SAMPLES"

    MEM_TOTAL=$(mem_total_mb)
    CPUS=$(nproc)
    DISK_TOTAL=$(df -m --output=size "$DISK_PATH" 2>/dev/null | tail -1 | tr -d ' ')
    DISK_TOTAL=${DISK_TOTAL:-0}

    read -r PEAK_MEM PEAK_SWAP PEAK_DISK PEAK_LOAD SAMPLE_COUNT <<< "$(
        awk -F'\t' 'NR>1 {
            if ($2+0 > m) m = $2+0
            if ($3+0 > s) s = $3+0
            if ($4+0 > d) d = $4+0
            if ($5+0 > l) l = $5+0
            n++
        } END { printf "%d %d %d %.2f %d", m, s, d, l, n }' "$SAMPLES"
    )"

    pct() { # $1 = part, $2 = whole -> integer percent, 0 when whole is 0
        [ "${2:-0}" -gt 0 ] 2>/dev/null && echo $(( $1 * 100 / $2 )) || echo 0
    }
    MEM_PCT=$(pct "$PEAK_MEM" "$MEM_TOTAL")
    DISK_PCT=$(pct "$PEAK_DISK" "$DISK_TOTAL")
    # Load is a float; compare in awk rather than shell arithmetic.
    LOAD_PCT=$(awk -v l="$PEAK_LOAD" -v c="$CPUS" 'BEGIN { printf "%d", (c>0 ? l*100/c : 0) }')

    echo "INFO: ---- resource peaks for '${LABEL}' (${SAMPLE_COUNT} samples) ----"
    echo "INFO: memory  ${PEAK_MEM} MB peak of ${MEM_TOTAL} MB provisioned (${MEM_PCT}%)"
    echo "INFO: swap    ${PEAK_SWAP} MB peak"
    echo "INFO: disk    ${PEAK_DISK} MB peak of ${DISK_TOTAL} MB on ${DISK_PATH} (${DISK_PCT}%)"
    echo "INFO: load1   ${PEAK_LOAD} peak across ${CPUS} vCPU (${LOAD_PCT}%)"

    VERDICT="sized about right"
    if [ "$MEM_PCT" -lt 25 ] && [ "$LOAD_PCT" -lt 50 ]; then
        VERDICT="**over-provisioned** - peak memory used under a quarter of the VM, and CPU never saturated"
    elif [ "$MEM_PCT" -gt 85 ] || [ "$DISK_PCT" -gt 85 ]; then
        VERDICT="**near a limit** - consider a larger plan"
    fi
    echo "INFO: verdict ${VERDICT}"

    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### Resource usage - ${LABEL}"
            echo ""
            echo "| Metric | Peak | Provisioned | Used |"
            echo "|---|---:|---:|---:|"
            echo "| Memory | ${PEAK_MEM} MB | ${MEM_TOTAL} MB | ${MEM_PCT}% |"
            echo "| Disk (${DISK_PATH}) | ${PEAK_DISK} MB | ${DISK_TOTAL} MB | ${DISK_PCT}% |"
            echo "| Load (1 min) | ${PEAK_LOAD} | ${CPUS} vCPU | ${LOAD_PCT}% |"
            echo "| Swap | ${PEAK_SWAP} MB | - | - |"
            echo ""
            echo "Verdict: ${VERDICT}"
            echo ""
            echo "<sub>Sampled every ${INTERVAL}s by \`.github/resource-monitor.sh\`. "
            echo "Memory is MemTotal-MemAvailable, so reclaimable page cache is not counted.</sub>"
        } >> "$GITHUB_STEP_SUMMARY"
    fi
    ;;

*)
    echo "usage: $0 {start|report} [label]" >&2
    ;;
esac

exit 0
