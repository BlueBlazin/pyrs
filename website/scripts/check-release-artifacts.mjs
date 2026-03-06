#!/usr/bin/env node

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import process from "node:process";

const scriptPath = fileURLToPath(import.meta.url);
const websiteRoot = path.resolve(path.dirname(scriptPath), "..");
const repoRoot = path.resolve(websiteRoot, "..");
const distRoot = path.join(websiteRoot, "dist");
const failures = [];

function walkFiles(rootDir) {
  const files = [];
  const stack = [rootDir];
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current || !fs.existsSync(current)) continue;
    const entries = fs.readdirSync(current, { withFileTypes: true });
    for (const entry of entries) {
      const absolutePath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(absolutePath);
      } else if (entry.isFile()) {
        files.push(absolutePath);
      }
    }
  }
  return files.sort();
}

if (!fs.existsSync(distRoot)) {
  failures.push("website/dist is missing; run the production build first");
} else {
  const files = walkFiles(distRoot);
  const disallowedExtensions = [".map", ".d.ts"];
  for (const file of files) {
    const relPath = path.relative(repoRoot, file);
    if (disallowedExtensions.some((ext) => file.endsWith(ext))) {
      failures.push(`disallowed release artifact found: ${relPath}`);
      continue;
    }
    if (!/\.(?:html|js|css)$/.test(file)) {
      continue;
    }
    const content = fs.readFileSync(file, "utf8");
    const disallowedMarkers = [
      "sourceMappingURL=",
      "import.meta.hot",
      "/@vite/client",
      "__vite",
      "localhost:",
    ];
    for (const marker of disallowedMarkers) {
      if (content.includes(marker)) {
        failures.push(`disallowed release marker '${marker}' found in ${relPath}`);
      }
    }
  }
}

if (failures.length > 0) {
  console.error("release artifact check failed:");
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log("release artifact check passed");
