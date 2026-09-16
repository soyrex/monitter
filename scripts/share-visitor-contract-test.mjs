import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const source = readFileSync("src/routes/share/+page.svelte", "utf8");

function matches(pattern, message) {
  assert.match(source.replace(/\s+/g, ''), new RegExp(pattern.source.replace(/\s+/g, ''), pattern.flags), message);
}

assert.ok(
  source.includes("new URLSearchParams(url.hash.replace(/^#/, ''))"),
  "fragment invite is supported",
);
assert.ok(
  source.includes(
    "fragment.get('invite') || rawInvite || url.searchParams.get('invite')",
  ),
  "query invite remains a legacy fallback",
);
matches(
  /url\.searchParams\.delete\('invite'\)[\s\S]*window\.history\.replaceState/,
  "invite token is removed with replaceState",
);
matches(
  /splitOperatorMessage\(message\.text\)/,
  "shared operator attribution parser is used",
);
matches(
  /participantColour\(item\.name\)/,
  "human bubbles use the stable participant hue",
);
matches(
  /applySharedAppearance\(next\.sharing\?\.appearance\)/,
  "live owner appearance is applied",
);
matches(
  /fragment\.get\('appearance'\)/,
  "fragment appearance fallback is accepted before connection",
);
matches(
  /storeAttachment\(taskId/,
  "attachments are stored through the scoped visitor session",
);
matches(
  /sendMessage\(taskId,text,sentAttachments\.map/,
  "attachment identifiers are included with visitor sends",
);
matches(
  /uploadLimits\.maxFileBytes/,
  "upload size is driven by shared capability metadata",
);
matches(
  /ondrop=\{dropFiles\}.*onpaste=\{pasteFiles\}/,
  "drag and paste file handlers are present",
);
matches(
  /aria-label="Read-only chat details"/,
  "chat details are explicitly read-only",
);
matches(
  /message\.senderAgentId.*snapshot\?\.agents\.find/,
  "assistant messages resolve their agent name",
);
matches(
  /next\.tasks\.length/,
  "a sole shared chat opens automatically",
);
matches(
  /delivery:'sending'/,
  "send adds an optimistic local message",
);
matches(
  /delivery:'uncertain'/,
  "failed sends are visibly not confirmed",
);
matches(
  /if\(draft\.trim\(\)===text\)draft=''/,
  "new typing is preserved while the captured send is in flight",
);
matches(
  /baselineIds=new Set/,
  "optimistic messages capture a pre-send baseline",
);
matches(
  /parsed\?\.text===item\.text&&\(parsed\.name\?\?''\)===item\.name/,
  "reconciliation requires exact author and text",
);
matches(
  /!item\.baselineIds\.has\(message\.id\)/,
  "old identical messages cannot consume the optimistic entry",
);
matches(
  /message\.role==='user'\?splitOperatorMessage/,
  "only human user messages are parsed for operator tags",
);
matches(
  /if\(following\)scrollLatest\(\)/,
  "refresh follows only while the reader remains at the bottom",
);
console.log("Share visitor contract passed.");
