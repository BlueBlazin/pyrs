const SCHEME_RE = /^[a-z][a-z\d+\-.]*:/i;

const normalizeBase = (base: string): string => {
	if (!base || base === "/") {
		return "/";
	}

	let normalized = base;
	if (!normalized.startsWith("/")) {
		normalized = `/${normalized}`;
	}
	if (!normalized.endsWith("/")) {
		normalized = `${normalized}/`;
	}
	return normalized;
};

const isSpecialHref = (href: string): boolean => {
	return (
		href.startsWith("#") ||
		href.startsWith("//") ||
		href.startsWith("mailto:") ||
		href.startsWith("tel:") ||
		href.startsWith("javascript:") ||
		href.startsWith("data:") ||
		href.startsWith("blob:") ||
		SCHEME_RE.test(href)
	);
};

const ensureLeadingSlash = (value: string): string => (value.startsWith("/") ? value : `/${value}`);

export const withBasePath = (href: string, base = import.meta.env.BASE_URL || "/"): string => {
	if (!href || isSpecialHref(href) || !href.startsWith("/")) {
		return href;
	}
	const normalizedBase = normalizeBase(base);
	if (normalizedBase === "/" || href.startsWith(normalizedBase)) {
		return href;
	}
	return `${normalizedBase}${href.slice(1)}`;
};

export const stripBasePath = (pathname: string, base = import.meta.env.BASE_URL || "/"): string => {
	const normalizedPath = ensureLeadingSlash(pathname || "/");
	const normalizedBase = normalizeBase(base);
	if (normalizedBase === "/") {
		return normalizedPath;
	}

	const baseWithoutTrailingSlash = normalizedBase.slice(0, -1);
	if (normalizedPath === baseWithoutTrailingSlash) {
		return "/";
	}
	if (normalizedPath.startsWith(normalizedBase)) {
		const stripped = normalizedPath.slice(normalizedBase.length - 1);
		return ensureLeadingSlash(stripped || "/");
	}
	return normalizedPath;
};
