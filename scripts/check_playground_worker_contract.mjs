#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const repoRoot = process.cwd();

const checks = [
  {
    file: "website/src/pages/playground.astro",
    label: "playground worker entrypoint dataset",
    patterns: [
      "data-worker-entrypoint={workerEntrypoint}",
      "new Worker(workerEntrypoint, { type: \"module\" })",
      "sendWorkerRequest(\"load\"",
      "sendWorkerRequest(\"execute\"",
      "sendWorkerRequest(\"reset\"",
    ],
  },
  {
    file: "website/public/workers/playground-runtime-worker.js",
    label: "worker action contract",
    patterns: [
      "action === \"load\"",
      "action === \"execute\"",
      "action === \"reset\"",
      "requestId",
      "self.postMessage({",
    ],
  },
];

const failures = [];

for (const check of checks) {
  const absolutePath = path.join(repoRoot, check.file);
  if (!fs.existsSync(absolutePath)) {
    failures.push(`${check.label}: missing file ${check.file}`);
    continue;
  }

  const content = fs.readFileSync(absolutePath, "utf8");
  for (const pattern of check.patterns) {
    if (!content.includes(pattern)) {
      failures.push(`${check.label}: missing pattern '${pattern}' in ${check.file}`);
    }
  }
}

if (failures.length > 0) {
  console.error("playground worker contract check failed:");
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log("playground worker contract check passed");
