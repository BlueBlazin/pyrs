import { promises as fs } from "node:fs";
import path from "node:path";

const rootDir = process.cwd();
const distDir = path.join(rootDir, "dist");

const baseRaw = process.env.ASTRO_BASE || "/";
const normalizedBase = (() => {
	if (!baseRaw || baseRaw === "/") {
		return "/";
	}
	let value = baseRaw;
	if (!value.startsWith("/")) {
		value = `/${value}`;
	}
	if (!value.endsWith("/")) {
		value = `${value}/`;
	}
	return value;
})();

const walk = async (dir) => {
	const entries = await fs.readdir(dir, { withFileTypes: true });
	const files = [];
	for (const entry of entries) {
		const full = path.join(dir, entry.name);
		if (entry.isDirectory()) {
			files.push(...(await walk(full)));
		} else {
			files.push(full);
		}
	}
	return files;
};

const exists = async (target) => {
	try {
		await fs.access(target);
		return true;
	} catch {
		return false;
	}
};

const isExternal = (value) => {
	return (
		value.startsWith("http://") ||
		value.startsWith("https://") ||
		value.startsWith("//") ||
		value.startsWith("mailto:") ||
		value.startsWith("tel:") ||
		value.startsWith("javascript:") ||
		value.startsWith("data:") ||
		value.startsWith("blob:")
	);
};

const stripQueryAndHash = (value) => {
	const [withoutHash] = value.split("#", 1);
	const [withoutQuery] = withoutHash.split("?", 1);
	return withoutQuery;
};

const htmlRouteDir = (htmlFile) => {
	const rel = path.relative(distDir, htmlFile).split(path.sep).join("/");
	if (rel === "index.html") {
		return "/";
	}
	if (rel.endsWith("/index.html")) {
		const prefix = rel.slice(0, -"/index.html".length);
		return `/${prefix}/`;
	}
	const dirname = path.dirname(rel);
	return dirname === "." ? "/" : `/${dirname}/`;
};

const applyBasePrefixStrip = (pathname) => {
	if (normalizedBase === "/") {
		return pathname;
	}
	if (pathname.startsWith(normalizedBase)) {
		const stripped = pathname.slice(normalizedBase.length - 1);
		return stripped.startsWith("/") ? stripped : `/${stripped}`;
	}
	return pathname;
};

const resolvePathname = (linkValue, htmlFile) => {
	const clean = stripQueryAndHash(linkValue.trim());
	if (!clean || clean.startsWith("#")) {
		return null;
	}
	if (clean.startsWith("/")) {
		return applyBasePrefixStrip(clean);
	}
	const origin = "https://example.com";
	const baseDir = htmlRouteDir(htmlFile);
	return new URL(clean, `${origin}${baseDir}`).pathname;
};

const candidateFiles = (pathname) => {
	const rel = pathname.replace(/^\/+/, "");
	const ext = path.extname(rel);
	if (ext) {
		return [path.join(distDir, rel)];
	}
	if (pathname.endsWith("/")) {
		return [path.join(distDir, rel, "index.html")];
	}
	return [
		path.join(distDir, rel),
		path.join(distDir, `${rel}.html`),
		path.join(distDir, rel, "index.html"),
	];
};

const parseInternalLinks = (html) => {
	const results = [];
	const regex = /\b(?:href|src)=["']([^"'<>]+)["']/g;
	let match = regex.exec(html);
	while (match) {
		results.push(match[1]);
		match = regex.exec(html);
	}
	return results;
};

const parseHeadChecks = (html) => {
	const issues = [];

	const titleMatch = html.match(/<title>([\s\S]*?)<\/title>/i);
	if (!titleMatch || !titleMatch[1] || titleMatch[1].trim().length === 0) {
		issues.push("missing or empty <title>");
	}

	const metaDescriptionTag = html.match(/<meta[^>]*name=["']description["'][^>]*>/i);
	if (!metaDescriptionTag) {
		issues.push("missing meta description");
	} else {
		const contentMatch = metaDescriptionTag[0].match(/content=["']([^"']*)["']/i);
		if (!contentMatch || contentMatch[1].trim().length === 0) {
			issues.push("empty meta description content");
		}
	}

	return issues;
};

const parseDocsShellChecks = (html, sourceRelPath) => {
	const issues = [];
	if (!sourceRelPath.startsWith("docs/")) {
		return issues;
	}

	const asideMatch = html.match(/<aside class=["']docs-sidebar["'][\s\S]*?<\/aside>/i);
	if (!asideMatch) {
		issues.push("missing docs sidebar container");
		return issues;
	}

	const sidebarHtml = asideMatch[0];
	if (!/<details[^>]*class=["'][^"']*sidebar-details[^"']*["'][^>]*\bopen\b/i.test(sidebarHtml)) {
		issues.push("docs sidebar details missing default open state");
	}

	const linkCount = (sidebarHtml.match(/<a\b/gi) || []).length;
	if (linkCount === 0) {
		issues.push("docs sidebar has no navigation links");
	}

	return issues;
};

const main = async () => {
	if (!(await exists(distDir))) {
		console.error(`[link-check] missing dist directory: ${distDir}`);
		process.exit(1);
	}

	const allFiles = await walk(distDir);
	const htmlFiles = allFiles.filter((file) => file.endsWith(".html"));
	const failures = [];

	for (const htmlFile of htmlFiles) {
		const html = await fs.readFile(htmlFile, "utf8");
		const sourceRelPath = path.relative(distDir, htmlFile).split(path.sep).join("/");
		const headIssues = parseHeadChecks(html);
		for (const issue of headIssues) {
			failures.push({
				source: sourceRelPath,
				link: "",
				pathname: "",
				issue,
			});
		}
		const docsShellIssues = parseDocsShellChecks(html, sourceRelPath);
		for (const issue of docsShellIssues) {
			failures.push({
				source: sourceRelPath,
				link: "",
				pathname: "",
				issue,
			});
		}

		const links = parseInternalLinks(html);
		for (const link of links) {
			if (isExternal(link) || link.startsWith("#")) {
				continue;
			}
			const pathname = resolvePathname(link, htmlFile);
			if (!pathname) {
				continue;
			}
			const candidates = candidateFiles(pathname);
			let found = false;
			for (const candidate of candidates) {
				if (await exists(candidate)) {
					found = true;
					break;
				}
			}
			if (!found) {
				failures.push({
					source: sourceRelPath,
					link,
					pathname,
					issue: "broken internal link",
				});
			}
		}
	}

	if (failures.length > 0) {
		console.error("[link-check] validation failures found:");
		for (const failure of failures) {
			if (failure.issue === "broken internal link") {
				console.error(`- ${failure.source}: ${failure.link} -> ${failure.pathname}`);
			} else {
				console.error(`- ${failure.source}: ${failure.issue}`);
			}
		}
		process.exit(1);
	}

	console.log(`[link-check] ok: ${htmlFiles.length} html files checked`);
};

await main();
