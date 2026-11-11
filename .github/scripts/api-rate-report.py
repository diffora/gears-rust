#!/usr/bin/env python3
"""Find which CI steps hit GitHub API rate limits, and which limit they hit.

Run it locally, with your own `gh` login. Your token has its own budget
(5,000/h for a PAT), so this never spends the repository's GITHUB_TOKEN budget
(5,000/h here, shared by every job) that it is diagnosing. Job logs are cached under
~/.cache/gears-api-rate-report/, and so are the job lists of completed runs:
neither changes, so a rerun over the same window costs a few listing calls.

  api-rate-report.py failures --since 24h [--workflow ci.yml] [--all-jobs]
      Scan job logs of failed runs for rate-limit errors. Prints one TSV row
      per hit, then counts by class and by (class, step).
      --all-jobs also scans jobs that succeeded (continue-on-error steps and
      steps that only warn).
  api-rate-report.py probe --since 24h
      Print the RATE / RATE-RES series written by api-rate-probe.yml, with the
      `used` delta between samples in the same reset window.
  api-rate-report.py selftest
      Check the classifier against sample lines. No network.

Classes:
  primary-installation  GITHUB_TOKEN or GitHub App installation budget exhausted
  primary-user          a PAT's budget exhausted
  unauthenticated       a request without a token; the IP is the runner's egress
  secondary             secondary (concurrency / points per minute) limit
  throttled-429         HTTP 429 or a Retry-After without a specific message
  other                 any other line mentioning a rate limit
"""

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

DEFAULT_REPO = "constructorfabric/gears-rust"
CACHE = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "gears-api-rate-report"
PROBE_WORKFLOW = "api-rate-probe.yml"

# Order matters: the first match wins.
_IP = r"(?P<ip>\d{1,3}(?:\.\d{1,3}){3}|[0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7})"
_RULES = [
    ("primary-installation", re.compile(r"rate limit exceeded for installation", re.I)),
    ("primary-user", re.compile(r"rate limit exceeded for user", re.I)),
    ("unauthenticated", re.compile(r"rate limit exceeded for " + _IP, re.I)),
    ("secondary", re.compile(r"secondary rate limit|abuse detection", re.I)),
    ("throttled-429", re.compile(r"\bHTTP[ /]?(?:\d(?:\.\d)? )?429\b|status(?: code)?:? 429\b|429 Too Many Requests", re.I)),
    ("other", re.compile(r"rate.?limit(?:ed)? exceeded|exceeded (?:a|the) .*rate.?limit", re.I)),
]
# Lines that mention a rate limit without reporting a hit: our own probe output.
_IGNORE = re.compile(r"^(?:RATE(?:-RES)? ts=|x-ratelimit-\w+:)")
_ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
# The runner echoes a run: script in cyan at the top of its step.
_ECHO = "\x1b[36;1m"
_TS = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.\d+)?Z ")
_STEP = re.compile(r"##\[group\]Run (.+)")


def classify(line):
    """Return (class, ip) for a log line that reports a rate-limit hit, else None."""
    if _IGNORE.search(line):
        return None
    for name, rx in _RULES:
        m = rx.search(line)
        if m:
            return name, (m.groupdict().get("ip") or "")
    return None


def step_at(steps, ts):
    """Name of the job step (from the jobs API) running at log timestamp ts.

    The API keeps whole seconds, so short steps overlap; None when ambiguous.
    """
    names = [st["name"] for st in steps or []
             if (st.get("started_at") or "z")[:19] <= ts <= ((st.get("completed_at") or "")[:19] or "9999")]
    return names[0] if len(names) == 1 else None


def scan_log(text, steps=None):
    """Yield (timestamp, step, class, ip, line) for each hit in a job log."""
    group = ""
    for raw in text.splitlines():
        m = _TS.match(raw)
        ts = m.group(1) if m else ""
        line = raw[m.end():] if m else raw
        if line.startswith(_ECHO):
            continue
        line = _ANSI.sub("", line)
        s = _STEP.match(line)
        if s:
            group = s.group(1).strip()[:80]
            # An action ref (owner/repo@ref, ./local) names itself; a script is
            # named by its first line, which is not the step name.
            if not re.match(r"(?:\./|[\w.-]+/[\w./-]+@)", group):
                group = "run: " + group
            continue
        step = step_at(steps, ts) or group
        hit = classify(line)
        if hit:
            yield ts, step, hit[0], hit[1], line.strip()[:200]


def cache_write(path, data):
    """Write a cache file atomically: an interrupted run must not leave a
    truncated log or job list that later runs would trust."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".tmp-", delete=False) as tmp:
        tmp.write(data)
    os.replace(tmp.name, path)


def gh_json(path, paginate=False):
    """GET a REST path through gh; with paginate, return the list of pages."""
    cmd = ["gh", "api", "-H", "Accept: application/vnd.github+json", path]
    if paginate:
        cmd[2:2] = ["--paginate", "--slurp"]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def job_log(repo, job_id):
    path = CACHE / repo.replace("/", "_") / f"{job_id}.log"
    if path.exists():
        return path.read_text(errors="replace")
    cmd = ["gh", "api", "--allow-escape-sequences", f"repos/{repo}/actions/jobs/{job_id}/logs"]
    res = subprocess.run(cmd, capture_output=True)
    if res.returncode != 0 and b"unknown flag" in res.stderr:
        # gh before --allow-escape-sequences existed prints logs as they are.
        del cmd[2]
        res = subprocess.run(cmd, capture_output=True)
    if res.returncode != 0:
        # Expired or deleted logs return 404/410: skip, do not cache.
        print(f"warning: no log for job {job_id}: {res.stderr.decode(errors='replace').strip()}",
              file=sys.stderr)
        return ""
    cache_write(path, res.stdout)
    return res.stdout.decode(errors="replace")


def since_arg(value):
    m = re.fullmatch(r"(\d+)([hd])", value)
    if not m:
        raise argparse.ArgumentTypeError("use <N>h or <N>d, e.g. 24h")
    hours = int(m.group(1)) * (24 if m.group(2) == "d" else 1)
    return dt.datetime.now(dt.timezone.utc) - dt.timedelta(hours=hours)


def list_runs(repo, since, workflow=None, status=None):
    created = since.strftime("%Y-%m-%dT%H:%M:%SZ")
    base = f"repos/{repo}/actions/workflows/{workflow}/runs" if workflow else f"repos/{repo}/actions/runs"
    query = f"?per_page=100&created=%3E%3D{created}" + (f"&status={status}" if status else "")
    return [r for page in gh_json(base + query, paginate=True) for r in page["workflow_runs"]]


def list_jobs(repo, run):
    """Jobs of the run's latest attempt; cached once the run has completed."""
    path = CACHE / repo.replace("/", "_") / f"run-{run['id']}-{run.get('run_attempt', 1)}.json"
    if path.exists():
        return json.loads(path.read_text())
    api = f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100&filter=latest"
    jobs = [j for page in gh_json(api, paginate=True) for j in page["jobs"]]
    if run.get("status") == "completed":
        cache_write(path, json.dumps(jobs).encode())
    return jobs


def cmd_failures(args):
    status = None if args.all_jobs else "failure"
    runs = list_runs(args.repo, args.since, args.workflow, status)
    print(f"# {len(runs)} runs since {args.since:%Y-%m-%d %H:%M}Z", file=sys.stderr)
    with ThreadPoolExecutor(max_workers=8) as pool:
        job_lists = list(pool.map(lambda r: list_jobs(args.repo, r), runs))
        jobs = [(run, job) for run, js in zip(runs, job_lists) for job in js
                if job.get("status") == "completed"
                and (args.all_jobs or job.get("conclusion") == "failure")]
        logs = list(pool.map(lambda rj: job_log(args.repo, rj[1]["id"]), jobs))
    print(f"# {len(jobs)} jobs scanned", file=sys.stderr)
    by_class, by_step = Counter(), Counter()
    print("\t".join(["time", "workflow", "job", "runner", "step", "class", "ip", "url"]))
    for (run, job), log in zip(jobs, logs):
        runner = ",".join(job.get("labels") or []) or job.get("runner_name") or ""
        seen = set()
        for ts, step, cls, ip, _line in scan_log(log, job.get("steps")):
            # One row per (step, class, ip) per job: retries repeat the same error.
            if (step, cls, ip) in seen:
                continue
            seen.add((step, cls, ip))
            by_class[cls] += 1
            by_step[(cls, step)] += 1
            print("\t".join([ts, run["name"], job["name"], runner, step, cls, ip, job["html_url"]]))
    print("\n# hits (one per job, step, class) by class")
    for cls, n in by_class.most_common():
        print(f"{n:6}  {cls}")
    print("\n# by class and step")
    for (cls, step), n in by_step.most_common():
        print(f"{n:6}  {cls:22}  {step}")


_KV = re.compile(r"(\w+)=(\S+)")


def cmd_probe(args):
    rows = []
    for run in list_runs(args.repo, args.since, PROBE_WORKFLOW):
        if run.get("status") != "completed":
            continue
        for job in list_jobs(args.repo, run):
            for raw in job_log(args.repo, job["id"]).splitlines():
                m = _TS.match(raw)
                line = raw[m.end():] if m else raw
                if line.startswith("RATE ") or line.startswith("RATE-RES "):
                    kv = dict(_KV.findall(line))
                    kv.setdefault("resource", "core")
                    rows.append(kv)
    rows.sort(key=lambda r: (r["resource"], r["ts"]))
    print("\t".join(["ts", "resource", "limit", "used", "remaining", "reset", "dused"]))
    prev = {}
    for r in rows:
        p = prev.get(r["resource"])
        delta = int(r["used"]) - int(p["used"]) if p and p["reset"] == r["reset"] else ""
        prev[r["resource"]] = r
        print("\t".join([r["ts"], r["resource"], r["limit"], r["used"], r["remaining"], r["reset"], str(delta)]))


SAMPLES = [
    ("##[error]API rate limit exceeded for installation ID 12345678.", "primary-installation", ""),
    ("Error: HttpError: API rate limit exceeded for installation ID 1", "primary-installation", ""),
    ("API rate limit exceeded for user ID 42.", "primary-user", ""),
    ("API rate limit exceeded for 203.0.113.7. (But here's the good news: ...)", "unauthenticated", "203.0.113.7"),
    ("API rate limit exceeded for 2001:db8::1. (But here's ...)", "unauthenticated", "2001:db8::1"),
    ("You have exceeded a secondary rate limit. Please wait a few minutes", "secondary", ""),
    ("gh: HTTP 429: Too Many Requests", "throttled-429", ""),
    ("::warning::could not retitle check 'x': HTTP Error 403: rate limit exceeded", "other", ""),
    ("Package 'php-symfony-rate-limiter' is not installed, so not removed", None, None),
    ("PASS [ 0.1s] cf-gears-oagw::e2e e2e_rate_limit_returns_429", None, None),
    ("test_rate_limited_error_code PASSED", None, None),
    ("'access-control-expose-headers': 'ETag, Retry-After, X-RateLimit-Limit'", None, None),
    ("RATE ts=2026-11-10T00:00:00Z limit=1000 used=10 remaining=990 reset=x", None, None),
    ("x-ratelimit-remaining: 0", None, None),
    ("Compiling ratatui v0.29.0", None, None),
]


def cmd_selftest(_args):
    bad = 0
    for line, cls, ip in SAMPLES:
        got = classify(line)
        want = (cls, ip) if cls else None
        if got != want:
            bad += 1
            print(f"FAIL {line!r}: got {got}, want {want}")
    log = ("2026-11-10T10:00:00.1Z ##[group]Run dorny/paths-filter@abc\n"
           "2026-11-10T10:00:01.2Z \x1b[36;1m# rate limit exceeded in a comment\x1b[0m\n"
           "2026-11-10T10:00:01.2Z ##[error]API rate limit exceeded for installation ID 1.\n")
    hits = [(h[0], h[1], h[2]) for h in scan_log(log)]
    if hits != [("2026-11-10T10:00:01", "dorny/paths-filter@abc", "primary-installation")]:
        bad += 1
        print(f"FAIL scan_log: {hits}")
    steps = [{"name": "Detect changed paths", "started_at": "2026-11-10T10:00:00Z",
              "completed_at": "2026-11-10T10:00:02Z"}]
    overlapping = steps + [{"name": "Next", "started_at": "2026-11-10T10:00:01Z",
                            "completed_at": "2026-11-10T10:00:01Z"}]
    hits = [h[1] for h in scan_log(log, steps)] + [h[1] for h in scan_log(log, overlapping)]
    if hits != ["Detect changed paths", "dorny/paths-filter@abc"]:
        bad += 1
        print(f"FAIL scan_log: {hits}")
    print("ok" if not bad else f"{bad} failed")
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", default=DEFAULT_REPO)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("failures")
    f.add_argument("--since", type=since_arg, default="24h")
    f.add_argument("--workflow", help="workflow file name, e.g. ci.yml")
    f.add_argument("--all-jobs", action="store_true")
    f.set_defaults(fn=cmd_failures)
    p = sub.add_parser("probe")
    p.add_argument("--since", type=since_arg, default="24h")
    p.set_defaults(fn=cmd_probe)
    sub.add_parser("selftest").set_defaults(fn=cmd_selftest)
    args = ap.parse_args()
    if isinstance(getattr(args, "since", None), str):
        args.since = since_arg(args.since)
    return args.fn(args) or 0


if __name__ == "__main__":
    sys.exit(main())
