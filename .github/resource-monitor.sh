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

LABEL="${2:-${GITHUB_JOB:-job}}"
STATE_DIR="${RESOURCE_MONITOR_DIR:-/var/tmp/resource-monitor}"
SAMPLES="${STATE_DIR}/${LABEL}.tsv"
PIDFILE="${STATE_DIR}/${LABEL}.pid"
BASEFILE="${STATE_DIR}/${LABEL}.baseline"
INTERVAL="${RESOURCE_MONITOR_INTERVAL:-10}"
# Defaults to the workspace, which is right for build jobs (cargo target dir).
# Jobs that write their bulk data elsewhere MUST set RESOURCE_MONITOR_DISK_PATH
# -- runtime-upgrade uses /mnt, the compatibility workflows /var/tmp. Watching
# the wrong path silently reports ~0% once that data lands on its own volume.
DISK_PATH="${RESOURCE_MONITOR_DISK_PATH:-${GITHUB_WORKSPACE:-$PWD}}"

# One awk pass over /proc/meminfo -> "used_mb total_mb swap_used_mb".
# used = MemTotal-MemAvailable, which is what the workload actually needs;
# "used" as reported by free() counts reclaimable page cache against you.
mem_stats() {
    awk '/^MemTotal:/ {t=$2} /^MemAvailable:/ {a=$2}
         /^SwapTotal:/ {st=$2} /^SwapFree:/ {sf=$2}
         END { printf "%d %d %d", int((t-a)/1024), int(t/1024), int((st-sf)/1024) }' \
        /proc/meminfo
}

# Cumulative busy/total jiffies -> "busy total". Deltas between two readings
# give real CPU utilisation. loadavg is a 60s EMA and cannot see a burst
# shorter than ~45s, which is the shape of most jobs here, so it is not used
# for the verdict. iowait counts as idle on purpose: a job waiting on disk is
# disk-bound, not CPU-bound, and should be reported that way.
cpu_jiffies() {
    awk '/^cpu / { idle = $5 + $6; total = 0
                   for (i = 2; i <= NF; i++) total += $i
                   printf "%d %d", total - idle, total; exit }' /proc/stat
}

disk_used_mb() { df -m --output=used "$DISK_PATH" 2>/dev/null | tail -1 | tr -d ' '; }
disk_size_mb() { df -m --output=size "$DISK_PATH" 2>/dev/null | tail -1 | tr -d ' '; }

# $1 = mem %, $2 = disk %, $3 = cpu %
#
# Order matters: these conditions are not mutually exclusive. A job can be
# memory-light and CPU-idle while filling its disk -- live-sync-creditcoin
# expands a 114 GB mainnet snapshot to 158 GB while needing little RAM. Linode
# bundles disk with RAM (g6-standard-6/8/16 = 320/640/1280 GB), so "shrink the
# plan" on memory evidence alone would cut the disk such a job depends on.
# Disk is checked first, and over-provisioned requires low disk as well.
verdict() {
    if [ "$1" -gt 85 ] || [ "$2" -gt 85 ]; then
        echo "**near a limit** - consider a larger plan"
    elif [ "$2" -gt 50 ] && [ "$1" -lt 25 ]; then
        echo "**disk-bound** - sized for its disk, not its RAM; do not shrink on memory alone"
    elif [ "$1" -lt 25 ] && [ "$3" -lt 50 ] && [ "$2" -le 50 ]; then
        echo "**over-provisioned** - peak memory under a quarter of the VM, CPU never saturated, disk at most half"
    else
        echo "sized about right"
    fi
}

case "${1:-}" in
start)
    mkdir -p "$STATE_DIR" || exit 0
    printf 'epoch\tmem_used_mb\tswap_used_mb\tdisk_used_mb\tcpu_pct\n' > "$SAMPLES"
    # Disk baseline: the runner image and any earlier job on this VM already
    # occupy space that is not this job's doing. Peak stays absolute (the plan
    # has to hold it) but the delta says what the job itself needed.
    disk_used_mb > "$BASEFILE"
    (
        # stdio must be detached. Inheriting the step's pipe makes
        # actions/runner wait out its full 5s "STDIO streams did not close"
        # grace period at the end of every start step.
        read -r prev_busy prev_total < <(cpu_jiffies)
        while true; do
            sleep "$INTERVAL"
            read -r busy total < <(cpu_jiffies)
            if [ "$total" -gt "$prev_total" ]; then
                cpu=$(( (busy - prev_busy) * 100 / (total - prev_total) ))
            else
                cpu=0
            fi
            prev_busy=$busy
            prev_total=$total
            read -r used _ swap < <(mem_stats)
            printf '%s\t%s\t%s\t%s\t%s\n' \
                "$(date -u +%s)" "$used" "$swap" "$(disk_used_mb)" "$cpu" >> "$SAMPLES"
        done
    ) </dev/null >/dev/null 2>&1 &
    echo $! > "$PIDFILE"
    echo "INFO: resource-monitor started for '${LABEL}' (pid $(cat "$PIDFILE"), every ${INTERVAL}s, disk ${DISK_PATH})"
    ;;

report)
    if [ -f "$PIDFILE" ]; then
        kill "$(cat "$PIDFILE")" 2>/dev/null
        rm -f "$PIDFILE"
    fi
    if [ ! -f "$SAMPLES" ]; then
        echo "WARNING: no sample file for '${LABEL}' - did the start step run with the same label?"
        exit 0
    fi

    # One final sample over a 1s window so a job shorter than the interval
    # still yields a real CPU figure rather than a fabricated zero.
    read -r b0 t0 < <(cpu_jiffies)
    sleep 1
    read -r b1 t1 < <(cpu_jiffies)
    if [ "$t1" -gt "$t0" ]; then
        final_cpu=$(( (b1 - b0) * 100 / (t1 - t0) ))
    else
        final_cpu=0
    fi
    read -r final_used MEM_TOTAL final_swap < <(mem_stats)
    printf '%s\t%s\t%s\t%s\t%s\n' \
        "$(date -u +%s)" "$final_used" "$final_swap" "$(disk_used_mb)" "$final_cpu" >> "$SAMPLES"

    CPUS=$(nproc)
    DISK_TOTAL=$(disk_size_mb); DISK_TOTAL=${DISK_TOTAL:-0}
    DISK_BASE=$(cat "$BASEFILE" 2>/dev/null); DISK_BASE=${DISK_BASE:-0}

    read -r PEAK_MEM PEAK_SWAP PEAK_DISK PEAK_CPU SAMPLE_COUNT <<< "$(
        awk -F'\t' 'NR>1 {
            if ($2+0 > m) m = $2+0
            if ($3+0 > s) s = $3+0
            if ($4+0 > d) d = $4+0
            if ($5+0 > c) c = $5+0
            n++
        } END { printf "%d %d %d %d %d", m, s, d, c, n }' "$SAMPLES"
    )"

    pct() { # $1 = part, $2 = whole -> integer percent, 0 when whole is 0
        [ "${2:-0}" -gt 0 ] 2>/dev/null && echo $(( $1 * 100 / $2 )) || echo 0
    }
    MEM_PCT=$(pct "$PEAK_MEM" "$MEM_TOTAL")
    DISK_PCT=$(pct "$PEAK_DISK" "$DISK_TOTAL")
    DISK_DELTA=$(( PEAK_DISK - DISK_BASE ))
    [ "$DISK_DELTA" -lt 0 ] && DISK_DELTA=0

    echo "INFO: ---- resource peaks for '${LABEL}' (${SAMPLE_COUNT} samples) ----"
    echo "INFO: memory  ${PEAK_MEM} MB peak of ${MEM_TOTAL} MB provisioned (${MEM_PCT}%)"
    echo "INFO: swap    ${PEAK_SWAP} MB peak"
    echo "INFO: disk    ${PEAK_DISK} MB peak of ${DISK_TOTAL} MB on ${DISK_PATH} (${DISK_PCT}%), ${DISK_DELTA} MB added by this job"
    echo "INFO: cpu     ${PEAK_CPU}% peak across ${CPUS} vCPU"

    # A verdict needs data, and only means something where the plan is ours to
    # choose. GitHub-hosted runner sizes are fixed, so there is nothing to resize.
    if [ "$SAMPLE_COUNT" -lt 2 ]; then
        VERDICT="not enough samples (${SAMPLE_COUNT}) - sampler may have died; treat peaks as unreliable"
    elif [ "${RUNNER_ENVIRONMENT:-}" = "github-hosted" ]; then
        VERDICT="n/a - GitHub-hosted runner, size is not ours to choose"
    else
        VERDICT=$(verdict "$MEM_PCT" "$DISK_PCT" "$PEAK_CPU")
    fi
    echo "INFO: verdict ${VERDICT}"

    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### Resource usage - ${LABEL}"
            echo ""
            echo "| Metric | Peak | Provisioned | Used |"
            echo "|---|---:|---:|---:|"
            echo "| Memory | ${PEAK_MEM} MB | ${MEM_TOTAL} MB | ${MEM_PCT}% |"
            echo "| Disk (\`${DISK_PATH}\`) | ${PEAK_DISK} MB | ${DISK_TOTAL} MB | ${DISK_PCT}% |"
            echo "| Disk added by this job | ${DISK_DELTA} MB | - | - |"
            echo "| CPU | ${PEAK_CPU}% | ${CPUS} vCPU | ${PEAK_CPU}% |"
            echo "| Swap | ${PEAK_SWAP} MB | - | - |"
            echo ""
            echo "Verdict: ${VERDICT}"
            echo ""
            echo "<sub>${SAMPLE_COUNT} samples at ${INTERVAL}s by \`.github/resource-monitor.sh\`. "
            echo "Memory is MemTotal-MemAvailable (reclaimable page cache not counted); "
            echo "CPU is busy jiffies from /proc/stat between samples, with iowait as idle.</sub>"
        } >> "$GITHUB_STEP_SUMMARY"
    fi
    ;;

*)
    echo "usage: $0 {start|report} [label]" >&2
    ;;
esac

exit 0
