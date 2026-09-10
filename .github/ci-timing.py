#!/usr/bin/env python3
"""Measure CI wall-clock time, so optimization work can be judged instead of guessed.

Pulls workflow/job durations from the GitHub Actions API via `gh`, and stores them
as a JSON snapshot. Take one snapshot before a change and one after, then compare.

    # before the change
    .github/ci-timing.py snapshot --out /tmp/before.json

    # ... land CI changes, let a few PRs run ...

    .github/ci-timing.py snapshot --out /tmp/after.json
    .github/ci-timing.py compare /tmp/before.json /tmp/after.json

`report` prints a single snapshot on its own.

Metrics reported:
  * wait      - per-commit end-to-end wait: first run queued to last run finished.
                This is the number a developer actually experiences.
  * workflow  - per-workflow duration (median / p90) and how often it fired.
  * job       - per-job duration inside a workflow, to locate the cost.

Durations are minutes. Queue time is included on purpose: waiting for a runner is
waiting. `--exclude-cancelled` drops cancelled runs, which are usually superseded
pushes rather than real work.
"""

import argparse
import json
import statistics
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timezone

# Only these events represent "a developer is waiting for CI on their change".
DEFAULT_EVENTS = ("pull_request",)


def gh_api(path):
    """Call the GitHub API through gh, returning one parsed JSON page.

    Deliberately not --paginate: the runs endpoint would walk the repo's entire
    history, which takes minutes. Callers page explicitly when they need to.
    """
    proc = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"gh api {path} failed:\n{proc.stderr.strip()}")
    return json.loads(proc.stdout)


def ts(value):
    if not value:
        return None
    return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()


def minutes(start, end):
    if start is None or end is None:
        return None
    return (end - start) / 60.0


def collect(limit, events, branch, with_jobs, exclude_cancelled):
    kept, page, max_pages = [], 1, 10
    while len(kept) < limit and page <= max_pages:
        # Filter server-side by event where possible so pages are not wasted.
        query = f"per_page=100&page={page}"
        if len(events) == 1:
            query += f"&event={events[0]}"
        if branch:
            query += f"&branch={branch}"
        batch = gh_api(f"repos/{{owner}}/{{repo}}/actions/runs?{query}").get("workflow_runs", [])
        if not batch:
            break
        for r in batch:
            if r["event"] not in events:
                continue
            if branch and r["head_branch"] != branch:
                continue
            if exclude_cancelled and r["conclusion"] == "cancelled":
                continue
            if r["status"] != "completed":
                continue
            kept.append(r)
            if len(kept) >= limit:
                break
        page += 1
    print(f"collected {len(kept)} runs; fetching job detail..." if with_jobs else
          f"collected {len(kept)} runs", file=sys.stderr)

    snapshot = {
        "captured_at": datetime.now(timezone.utc).isoformat(),
        "events": list(events),
        "branch": branch,
        "exclude_cancelled": exclude_cancelled,
        "runs": [],
    }

    for r in kept:
        entry = {
            "name": r["name"],
            "id": r["id"],
            "sha": r["head_sha"],
            "branch": r["head_branch"],
            "conclusion": r["conclusion"],
            "attempt": r["run_attempt"],
            # created_at -> updated_at spans queue + execution, which is the wait.
            "minutes": minutes(ts(r["created_at"]), ts(r["updated_at"])),
            "jobs": [],
        }
        if with_jobs:
            jobs = gh_api(
                f"repos/{{owner}}/{{repo}}/actions/runs/{r['id']}/jobs?per_page=100"
            ).get("jobs", [])
            for j in jobs:
                entry["jobs"].append(
                    {
                        "name": j["name"],
                        "conclusion": j["conclusion"],
                        "minutes": minutes(ts(j["started_at"]), ts(j["completed_at"])),
                        "queue_minutes": minutes(ts(j["created_at"]), ts(j["started_at"])),
                    }
                )
        snapshot["runs"].append(entry)

    return snapshot


def pct(values, q):
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[min(int(len(ordered) * q), len(ordered) - 1)]


def summarize(snap):
    """Reduce a snapshot to comparable aggregates."""
    by_wf = defaultdict(list)
    by_job = defaultdict(list)
    by_sha_span = defaultdict(lambda: [None, None])

    for r in snap["runs"]:
        if r["minutes"] is not None:
            by_wf[r["name"]].append(r["minutes"])
        for j in r["jobs"]:
            if j["minutes"] is not None:
                by_job[(r["name"], j["name"])].append(j["minutes"])

    # Per-commit wait needs absolute times, which only the raw runs carry; recompute
    # from per-run durations is not possible, so approximate the wait as the slowest
    # workflow on that commit. That is the critical path when workflows run in parallel.
    by_sha = defaultdict(list)
    for r in snap["runs"]:
        if r["minutes"] is not None:
            by_sha[r["sha"]].append(r["minutes"])
    waits = [max(v) for v in by_sha.values() if v]

    shas = len(by_sha) or 1
    return {
        "commits": len(by_sha),
        "runs": len(snap["runs"]),
        "wait": {
            "median": statistics.median(waits) if waits else 0.0,
            "p90": pct(waits, 0.9),
            "max": max(waits) if waits else 0.0,
        },
        "workflows": {
            name: {
                "median": statistics.median(v),
                "p90": pct(v, 0.9),
                "runs": len(v),
                "fire_rate": len(v) / shas,
            }
            for name, v in by_wf.items()
        },
        "jobs": {
            f"{wf} / {job}": {"median": statistics.median(v), "p90": pct(v, 0.9), "runs": len(v)}
            for (wf, job), v in by_job.items()
        },
    }


def print_report(snap, title="CI timing"):
    s = summarize(snap)
    print(f"\n{title}")
    print(f"  captured {snap['captured_at']}  commits={s['commits']}  runs={s['runs']}")
    w = s["wait"]
    print(
        f"  per-commit wait (slowest workflow): "
        f"median {w['median']:.0f}m  p90 {w['p90']:.0f}m  max {w['max']:.0f}m"
    )

    print(f"\n  {'workflow':<32}{'median':>8}{'p90':>8}{'fires':>8}{'runs':>7}")
    for name, v in sorted(s["workflows"].items(), key=lambda kv: -kv[1]["median"]):
        print(
            f"  {name[:31]:<32}{v['median']:7.0f}m{v['p90']:7.0f}m"
            f"{v['fire_rate'] * 100:7.0f}%{v['runs']:7d}"
        )

    if s["jobs"]:
        print(f"\n  {'slowest jobs':<52}{'median':>8}{'p90':>8}")
        top = sorted(s["jobs"].items(), key=lambda kv: -kv[1]["median"])[:20]
        for name, v in top:
            print(f"  {name[:51]:<52}{v['median']:7.0f}m{v['p90']:7.0f}m")
    print()


def print_compare(before, after):
    b, a = summarize(before), summarize(after)

    print(f"\nCI timing: before vs after")
    print(f"  before: {before['captured_at']}  commits={b['commits']}  runs={b['runs']}")
    print(f"  after:  {after['captured_at']}  commits={a['commits']}  runs={a['runs']}")

    print(f"\n  {'metric':<32}{'before':>9}{'after':>9}{'delta':>9}")
    for key in ("median", "p90", "max"):
        bv, av = b["wait"][key], a["wait"][key]
        print(f"  {'per-commit wait ' + key:<32}{bv:8.0f}m{av:8.0f}m{av - bv:+8.0f}m")

    def section(label, bd, ad):
        names = set(bd) | set(ad)
        rows = []
        for n in names:
            bv = bd.get(n, {}).get("median")
            av = ad.get(n, {}).get("median")
            if bv is None:
                rows.append((0.0, n, None, av, "new"))
            elif av is None:
                rows.append((-bv, n, bv, None, "gone"))
            else:
                rows.append((av - bv, n, bv, av, ""))
        if not rows:
            return
        print(f"\n  {label:<52}{'before':>9}{'after':>9}{'delta':>9}")
        for delta, n, bv, av, note in sorted(rows, key=lambda r: r[0]):
            bs = f"{bv:8.0f}m" if bv is not None else "       -"
            as_ = f"{av:8.0f}m" if av is not None else "       -"
            ds = f"{delta:+8.0f}m" if not note else f"{note:>9}"
            print(f"  {n[:51]:<52}{bs}{as_}{ds}")

    section("workflow (median)", b["workflows"], a["workflows"])
    section("job (median)", b["jobs"], a["jobs"])
    print(
        "\n  Caveat: PRs differ in what they touch, so compare like with like.\n"
        "  Prefer a wide window (--limit 200) or re-run the same PR before and after.\n"
    )


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    snap = sub.add_parser("snapshot", help="capture timings to a JSON file")
    snap.add_argument("--out", required=True)
    snap.add_argument("--limit", type=int, default=150, help="max runs to collect (default 150)")
    snap.add_argument("--branch", help="only runs for this head branch")
    snap.add_argument(
        "--events",
        default=",".join(DEFAULT_EVENTS),
        help="comma-separated run events (default pull_request)",
    )
    snap.add_argument("--no-jobs", action="store_true", help="skip per-job detail (much faster)")
    snap.add_argument(
        "--include-cancelled",
        action="store_true",
        help="keep cancelled runs (default drops them as superseded pushes)",
    )

    rep = sub.add_parser("report", help="print a snapshot")
    rep.add_argument("snapshot")

    cmp_ = sub.add_parser("compare", help="diff two snapshots")
    cmp_.add_argument("before")
    cmp_.add_argument("after")

    args = p.parse_args()

    if args.cmd == "snapshot":
        data = collect(
            limit=args.limit,
            events=tuple(e.strip() for e in args.events.split(",") if e.strip()),
            branch=args.branch,
            with_jobs=not args.no_jobs,
            exclude_cancelled=not args.include_cancelled,
        )
        with open(args.out, "w") as fh:
            json.dump(data, fh, indent=2)
        print_report(data, title=f"CI timing snapshot -> {args.out}")
    elif args.cmd == "report":
        with open(args.snapshot) as fh:
            print_report(json.load(fh), title=f"CI timing: {args.snapshot}")
    else:
        with open(args.before) as fh:
            before = json.load(fh)
        with open(args.after) as fh:
            after = json.load(fh)
        print_compare(before, after)


if __name__ == "__main__":
    main()
