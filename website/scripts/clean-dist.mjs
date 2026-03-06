#!/usr/bin/env node

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptPath = fileURLToPath(import.meta.url);
const websiteRoot = path.resolve(path.dirname(scriptPath), "..");
const distRoot = path.join(websiteRoot, "dist");

fs.rmSync(distRoot, { recursive: true, force: true });
console.log("cleaned website/dist");
