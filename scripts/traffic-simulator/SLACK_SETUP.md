# Slack Notifications Setup Guide

This guide explains how to set up Slack channel alerts and hourly sanity notifications for the traffic-simulator.

## Overview

The traffic-simulator now supports hourly Slack reports that include:
- Successful/failed proof counts
- Submission breakdown (single vs batch)
- Blocks processed
- Connection status
- Success rate
- Error information

## Quick Start

### Option 1: Built-in Reporting (Single Instance)

Enable Slack reporting directly in the simulator:

```bash
deno task start -- \
  --source-rpc wss://... \
  --private-key 0x... \
  --slack-webhook https://hooks.slack.com/services/YOUR/WEBHOOK/URL \
  --slack-alert-group S123456
```

The simulator will automatically send hourly reports.

### Option 2: Kubernetes CronJob (Production)

Deploy a separate CronJob that queries metrics and sends reports:

```bash
# 1. Edit k8s/cronjob-reporter.yaml with your Slack webhook URL
# 2. Deploy
kubectl apply -f k8s/cronjob-reporter.yaml
```

## Getting Slack Webhook URL

1. Go to https://api.slack.com/apps
2. Create a new app or select an existing one
3. Navigate to "Incoming Webhooks"
4. Activate incoming webhooks
5. Click "Add New Webhook to Workspace"
6. Select your channel
7. Copy the webhook URL (format: `https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX`)

## Getting Slack User/Group ID (Optional)

### User ID
- Open Slack
- Click on the user's profile
- The ID is in the URL: `https://workspace.slack.com/team/U123456`
- Format: `U123456`

### Group ID
- Go to Slack workspace settings
- Navigate to User Groups
- Click on the group
- The ID is in the URL: `https://workspace.slack.com/usergroups/S123456`
- Format: `S123456`

## Configuration

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `SLACK_WEBHOOK_URL` | Slack webhook URL | Yes |
| `SLACK_ALERT_GROUP` | User/group ID for mentions | No |

### CLI Arguments

| Argument | Description | Required |
|----------|-------------|----------|
| `--slack-webhook` | Slack webhook URL | No |
| `--slack-alert-group` | User/group ID for mentions | No |

## Report Format

Hourly reports include:

```
📊 Traffic Simulator Hourly Report

Period: 2025-01-28T10:00:00Z → 2025-01-28T11:00:00Z
Duration: 1.00 hours

Connection Status: ✅
  • Sepolia: ✅ Connected
  • CC3: ✅ Connected

Proof Submissions:
  • Successful: 1,234 (1,234.0/hr)
  • Failed: ✅ 0
  • Success Rate: 100.0%

Submission Breakdown:
  • Single: 500
  • Batch: 734

Blocks:
  • Processed: 1,234
  • Queue Size: 5

Totals (since start):
  • Proofs Submitted: 10,000
  • Proof Errors: 5
  • Blocks Processed: 10,000
  • Uptime: 2d 5h 30m
```

## Troubleshooting

### No Reports Received

1. **Check CronJob logs**:
   ```bash
   kubectl logs -l app=traffic-simulator-reporter --tail=50
   ```

2. **Verify webhook URL**: Ensure the URL is correct and not expired

3. **Check simulator accessibility**: Ensure the CronJob can reach the simulator service
   ```bash
   kubectl run -it --rm debug --image=curlimages/curl --restart=Never -- \
     curl http://traffic-simulator:8080/status
   ```

### Reports Missing Metrics

- Ensure the simulator has been running for at least 1 hour
- Check that the `/status` endpoint is accessible
- Verify metrics are being tracked (check simulator logs)

### Snapshot Resets

If using the non-persistent CronJob version, snapshots reset on pod restart. Use the persistent version:

```bash
kubectl apply -f k8s/cronjob-reporter-persistent.yaml
```

## Customization

### Change Report Frequency

Edit the CronJob schedule in `k8s/cronjob-reporter.yaml`:

```yaml
spec:
  schedule: "0 * * * *"  # Every hour
  # schedule: "0 */2 * * *"  # Every 2 hours
  # schedule: "0 9-17 * * *"  # Every hour during business hours (9 AM - 5 PM)
```

### Customize Report Format

Edit `src/slack.ts` to customize the report format and content.

## Files Created

- `src/slack.ts` - Slack notification utilities
- `src/reportSender.ts` - CronJob script for querying metrics and sending reports
- `k8s/cronjob-reporter.yaml` - Kubernetes CronJob manifest (non-persistent)
- `k8s/cronjob-reporter-persistent.yaml` - Kubernetes CronJob manifest (persistent)

## Integration Points

The Slack reporting integrates with:
- `/status` endpoint - Provides current metrics
- Metrics tracking in `main.ts` - Tracks proof submissions and errors
- Health status - Monitors connection health
