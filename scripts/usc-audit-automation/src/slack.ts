/**
 * Slack notification for USC audit reports
 */

import { fetchWithTimeout } from "./fetch.ts";

export type BuiltReport = {
  ok: boolean;
  summary: string;
  details: string;
};

export interface SlackPayload {
  text: string;
  blocks?: unknown[];
}

export function createSlackPayloads(
  report: BuiltReport,
  alertGroup?: string,
): { summaryPayload: SlackPayload; detailsPayload: SlackPayload } {
  const codeBlock = "```" + report.summary + "```";

  let summaryText = codeBlock;
  if (alertGroup && !report.ok) {
    const mention = alertGroup.startsWith("U")
      ? `<@${alertGroup}>`
      : alertGroup.startsWith("S")
      ? `<!subteam^${alertGroup}>`
      : alertGroup;
    summaryText = `${mention}\n\n${codeBlock}`;
  }

  const summaryPayload: SlackPayload = {
    text: summaryText,
  };

  let detailsText = "";
  if (report.details?.trim()) {
    detailsText = "```" + report.details + "```";
  }
  const detailsPayload: SlackPayload = {
    text: detailsText,
  };

  return { summaryPayload, detailsPayload };
}

export async function sendSummarySlackMessage(
  slackBotToken: string,
  channel: string,
  payload: SlackPayload,
): Promise<string> {
  const res = await fetchWithTimeout("https://slack.com/api/chat.postMessage", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${slackBotToken}`,
    },
    body: JSON.stringify({
      channel,
      ...payload,
    }),
  });

  const json = await res.json();

  if (!res.ok || !json.ok || !json.ts) {
    throw new Error(
      `Slack API failed: status=${res.status}, error=${
        json.error ?? "unknown_error"
      }`,
    );
  }

  return json.ts; // 👈 needed to reply to this message in a thread
}

export async function sendThreadSlackMessage(
  slackBotToken: string,
  channel: string,
  threadTs: string,
  payload: SlackPayload,
): Promise<void> {
  const res = await fetchWithTimeout("https://slack.com/api/chat.postMessage", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${slackBotToken}`,
    },
    body: JSON.stringify({
      channel,
      thread_ts: threadTs,
      ...payload,
    }),
  });

  const json = await res.json();

  if (!res.ok || !json.ok) {
    throw new Error(`Slack thread failed: ${json.error ?? res.status}`);
  }
}
