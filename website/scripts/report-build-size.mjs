import { promises as fs } from "node:fs";
import path from "node:path";

const rootDir = process.cwd();
const distDir = path.join(rootDir, "dist");
const outDir = path.join(rootDir, "perf");
const outPath = path.join(outDir, "build_size_latest.json");

const toPosix = (value) => value.split(path.sep).join("/");

const walk = async (dir) => {
	const entries = await fs.readdir(dir, { withFileTypes: true });
	const files = [];
	for (const entry of entries) {
		const full = path.join(dir, entry.name);
		if (entry.isDirectory()) {
			files.push(...(await walk(full)));
			continue;
		}
		files.push(full);
	}
	return files;
};

const classifyExt = (filePath) => {
	const ext = path.extname(filePath).toLowerCase();
	if (ext === ".html") return "html";
	if (ext === ".css") return "css";
	if (ext === ".js" || ext === ".mjs") return "js";
	if (ext === ".svg") return "svg";
	if ([".png", ".jpg", ".jpeg", ".webp", ".avif", ".gif"].includes(ext)) return "image";
	if ([".woff", ".woff2", ".ttf", ".otf"].includes(ext)) return "font";
	return "other";
};

const kib = (bytes) => Number((bytes / 1024).toFixed(2));

const main = async () => {
	try {
		await fs.access(distDir);
	} catch {
		console.error(`[size-report] missing dist directory: ${distDir}`);
		console.error("[size-report] run `pnpm --dir website build` first.");
		process.exit(1);
	}

	const files = await walk(distDir);
	const stats = [];
	for (const file of files) {
		const fileStat = await fs.stat(file);
		if (!fileStat.isFile()) {
			continue;
		}
		const rel = toPosix(path.relative(distDir, file));
		stats.push({
			path: rel,
			bytes: fileStat.size,
			type: classifyExt(file),
		});
	}

	const totals = {
		file_count: stats.length,
		total_bytes: stats.reduce((sum, entry) => sum + entry.bytes, 0),
		by_type: {},
	};
	for (const entry of stats) {
		totals.by_type[entry.type] = (totals.by_type[entry.type] || 0) + entry.bytes;
	}

	const largest_assets = [...stats]
		.sort((a, b) => b.bytes - a.bytes)
		.slice(0, 12)
		.map((entry) => ({
			path: entry.path,
			type: entry.type,
			bytes: entry.bytes,
			kib: kib(entry.bytes),
		}));

	const report = {
		generated_at_utc: new Date().toISOString(),
		dist_dir: distDir,
		summary: {
			file_count: totals.file_count,
			total_bytes: totals.total_bytes,
			total_kib: kib(totals.total_bytes),
			by_type: Object.fromEntries(
				Object.entries(totals.by_type).map(([type, bytes]) => [type, { bytes, kib: kib(bytes) }]),
			),
		},
		largest_assets,
	};

	await fs.mkdir(outDir, { recursive: true });
	await fs.writeFile(outPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");

	console.log(`[size-report] wrote ${outPath}`);
	console.log(
		`[size-report] total=${report.summary.total_kib} KiB across ${report.summary.file_count} files`,
	);
};

await main();
