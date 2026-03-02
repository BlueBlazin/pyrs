#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

function parseArgs(argv) {
  const parsed = {
    out: null,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--out") {
      parsed.out = argv[index + 1] || null;
      index += 1;
    } else {
      throw new Error(`unknown argument: ${token}`);
    }
  }
  return parsed;
}

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
const details = [];

for (const check of checks) {
  const absolutePath = path.join(repoRoot, check.file);
  if (!fs.existsSync(absolutePath)) {
    failures.push(`${check.label}: missing file ${check.file}`);
    details.push({
      file: check.file,
      label: check.label,
      missing_file: true,
      matched_patterns: [],
      missing_patterns: check.patterns,
    });
    continue;
  }

  const content = fs.readFileSync(absolutePath, "utf8");
  const matchedPatterns = [];
  const missingPatterns = [];
  for (const pattern of check.patterns) {
    if (!content.includes(pattern)) {
      failures.push(`${check.label}: missing pattern '${pattern}' in ${check.file}`);
      missingPatterns.push(pattern);
    } else {
      matchedPatterns.push(pattern);
    }
  }
  details.push({
    file: check.file,
    label: check.label,
    missing_file: false,
    matched_patterns: matchedPatterns,
    missing_patterns: missingPatterns,
  });
}

let args;
try {
  args = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(
    `playground worker contract check failed: ${error instanceof Error ? error.message : String(error)}`
  );
  process.exit(1);
}

if (args.out) {
  const payload = {
    ok: failures.length === 0,
    check_count: checks.length,
    failure_count: failures.length,
    failures,
    checks: details,
  };
  const outPath = path.resolve(repoRoot, args.out);
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  console.log(`wrote ${args.out}`);
}

if (failures.length > 0) {
  console.error("playground worker contract check failed:");
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log("playground worker contract check passed");
