import json
import os
import sys
import time
import urllib.parse
import urllib.request

MAX_RETRIES = 3
RETRY_DELAY_SECONDS = 2
TOKEN_SYMBOL = "CTC"
TOKEN_DECIMALS = 18
THRESHOLD_CTC = 10
THRESHOLD_WEI = THRESHOLD_CTC * (10**TOKEN_DECIMALS)


def fetch_balance(base_url, address):
    params = urllib.parse.urlencode(
        {"module": "account", "action": "balance", "address": address}
    )
    url = f"{base_url.rstrip('/')}/api?{params}"

    last_error = None
    for attempt in range(MAX_RETRIES):
        try:
            with urllib.request.urlopen(url, timeout=20) as resp:
                data = json.loads(resp.read().decode("utf-8"))

            result = data.get("result")
            if isinstance(result, str) and result.isdigit():
                return int(result), None
            return None, f"unexpected response: {data}"
        except Exception as exc:
            last_error = exc
            if attempt < MAX_RETRIES - 1:
                time.sleep(RETRY_DELAY_SECONDS)

    return None, f"request failed after {MAX_RETRIES} attempts: {last_error}"


def main():
    networks_path = os.environ.get("NETWORKS_JSON_PATH")
    if not networks_path:
        print("NETWORKS_JSON_PATH is required", file=sys.stderr)
        return 1
    with open(networks_path, "r", encoding="utf-8") as handle:
        networks = json.load(handle)
    webhook_url = os.environ.get("SLACK_WEBHOOK_URL")
    if not webhook_url:
        print("SLACK_WEBHOOK_URL is required", file=sys.stderr)
        return 1

    alert_groups = [
        g.strip()
        for g in os.environ.get("SLACK_ALERT_USER_GROUP_IDS", "").split(",")
        if g.strip()
    ]

    network_reports = []
    low_lines = []

    for net in networks:
        name = net.get("name", "unknown")
        base_url = net.get("base_url")
        accounts = net.get("accounts", [])
        if not base_url:
            network_reports.append((name, "missing base_url"))
            continue
        if not accounts:
            network_reports.append((name, "no accounts configured"))
            continue

        rows = []
        for account in accounts:
            if isinstance(account, str):
                addr = account
                label = ""
            else:
                addr = account.get("address")
                label = account.get("name", "")
            if not addr:
                rows.append(
                    {"display": str(account), "token": None, "err": "invalid account entry"}
                )
                continue

            display = f"{addr} ({label})" if label else addr
            bal, err = fetch_balance(base_url, addr)
            if err:
                rows.append({"display": display, "token": None, "err": err})
                continue

            token = bal / (10**TOKEN_DECIMALS)
            is_low = bal < THRESHOLD_WEI
            rows.append({"display": display, "token": token, "err": None, "is_low": is_low})
            if is_low:
                low_lines.append(f"- {name}, `{display}`: {token:.6f} {TOKEN_SYMBOL}")

        network_reports.append((name, rows))

    lines = ["*Daily balance report*"]
    if not network_reports:
        lines.append("- no accounts or networks configured")
    else:
        for name, rows in network_reports:
            if isinstance(rows, str):
                lines.append("```\n" + f"🟪 {name}\n{rows}\n```")
                continue

            if not rows:
                lines.append("```\n" + f"🟪 {name}\nno data\n```")
                continue

            valid_rows = [r for r in rows if r["token"] is not None]
            error_rows = [r for r in rows if r["token"] is None]

            table = []
            if valid_rows:
                status_width = len("Status")
                display_width = max(
                    len("Account"),
                    max(len(r["display"]) for r in valid_rows),
                )
                amount_width = max(
                    len("Balance"),
                    max(
                        len(f"{r['token']:.6f} {TOKEN_SYMBOL}")
                        for r in valid_rows
                    ),
                )
                table.append(
                    f"{'Status':<{status_width}}  "
                    f"{'Account':<{display_width}}  "
                    f"{'Balance':>{amount_width}}"
                )
                for r in valid_rows:
                    status = "❌" if r["is_low"] else "✅"
                    amount = f"{r['token']:.6f} {TOKEN_SYMBOL}"
                    table.append(
                        f"{status:<{status_width}}  "
                        f"{r['display']:<{display_width}}  "
                        f"{amount:>{amount_width}}"
                    )
            if error_rows:
                if table:
                    table.append("")
                for r in error_rows:
                    table.append(f"ERROR: {r['display']} - {r['err']}")

            lines.append("```\n" + f"🟪 {name}\n" + "\n".join(table) + "\n```")

    if low_lines:
        lines += ["", "*Low balance alert*"]
        lines.append(f"Threshold: {THRESHOLD_CTC} {TOKEN_SYMBOL}")
        lines.extend(low_lines)
        if alert_groups:
            mentions = " ".join(f"<!subteam^{g}>" for g in alert_groups)
            lines += ["", f"Notify: {mentions}"]

    payload = {"text": "\n".join(lines)}

    req = urllib.request.Request(
        webhook_url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=20) as resp:
        resp.read()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
