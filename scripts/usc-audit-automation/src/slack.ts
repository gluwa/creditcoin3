/**
 * Slack notification for USC audit reports
 */

import { fetchWithTimeout } from "./fetch.ts";

export interface SlackPayload {
  username: string;
  icon_emoji: string;
  text: string;
}

export function createSlackPayload(
  reportText: string,
  hasErrors: boolean,
  alertGroup?: string,
): SlackPayload {
  const codeBlock = "```" + reportText + "```";
  let text = codeBlock;
  if (alertGroup && hasErrors) {
    const mention = alertGroup.startsWith("U")
      ? `<@${alertGroup}>`
      : alertGroup.startsWith("S")
      ? `<!subteam^${alertGroup}>`
      : alertGroup;
    text = `${mention}\n\n${codeBlock}`;
  }

  return {
    username: "usc-audit-automation",
    icon_emoji: hasErrors ? ":rotating_light:" : ":shield:",
    text,
  };
}

export async function sendSlackMessage(
  webhookUrl: string,
  payload: SlackPayload,
): Promise<void> {
  const res = await fetchWithTimeout(webhookUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Slack webhook failed: ${res.status} - ${body}`);
  }
}
