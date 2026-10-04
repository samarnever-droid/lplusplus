export interface PackageItem {
  name: string;
  version: string;
  description?: string;
  authors?: string[];
  keywords?: string[];
  downloads?: number;
  updated_at?: string;
  owner?: string;
  organization?: string;
  dependencies?: string[];
  license?: string;
  sha256?: string;
  download_url?: string;
  readme?: string;
}

export type SortMode = "relevance" | "downloads" | "recent" | "name";

/**
 * High-performance package ranking & scoring algorithm:
 * - Exact name match: +1000 pts
 * - Prefix name match: +500 pts
 * - Name contains query: +200 pts
 * - Scope/Org matches: +150 pts
 * - Keyword exact match: +100 pts
 * - Keyword contains query: +50 pts
 * - Description match: +20 pts
 * - Author match: +15 pts
 * - Downloads boost: log10(downloads + 1) * 10
 */
export function rankPackages(
  packages: PackageItem[],
  query: string,
  categoryFilter?: string,
  sortMode: SortMode = "relevance"
): PackageItem[] {
  const q = query.trim().toLowerCase();
  const cat = categoryFilter?.trim().toLowerCase();

  let filtered = packages;

  // Filter by category / tag if specified
  if (cat && cat !== "all") {
    filtered = filtered.filter((pkg) => {
      const keywords = (pkg.keywords || []).map((k) => k.toLowerCase());
      return keywords.includes(cat) || pkg.name.toLowerCase().includes(cat);
    });
  }

  if (!q) {
    return sortPackageList([...filtered], sortMode);
  }

  const scored = filtered.map((pkg) => {
    let score = 0;
    const name = pkg.name.toLowerCase();
    const desc = (pkg.description || "").toLowerCase();
    const keywords = (pkg.keywords || []).map((k) => k.toLowerCase());
    const authors = (pkg.authors || []).map((a) => a.toLowerCase());
    const org = (pkg.organization || "").toLowerCase();

    // Exact name match
    if (name === q) score += 1000;
    // Name starts with query
    else if (name.startsWith(q)) score += 500;
    // Name contains query
    else if (name.includes(q)) score += 200;

    // Organization match
    if (org && (org === q || org.includes(q))) score += 150;

    // Keyword matches
    for (const kw of keywords) {
      if (kw === q) score += 100;
      else if (kw.includes(q)) score += 50;
    }

    // Description match
    if (desc.includes(q)) score += 20;

    // Author match
    for (const auth of authors) {
      if (auth.includes(q)) score += 15;
    }

    // Popularity weighting
    if (pkg.downloads && pkg.downloads > 0) {
      score += Math.log10(pkg.downloads + 1) * 10;
    }

    return { pkg, score };
  });

  // Filter out non-matches (score 0)
  const matches = scored.filter((item) => item.score > 0);

  if (sortMode === "relevance") {
    matches.sort((a, b) => b.score - a.score);
    return matches.map((m) => m.pkg);
  }

  return sortPackageList(matches.map((m) => m.pkg), sortMode);
}

function sortPackageList(list: PackageItem[], mode: SortMode): PackageItem[] {
  switch (mode) {
    case "downloads":
      return list.sort((a, b) => (b.downloads || 0) - (a.downloads || 0));
    case "recent":
      return list.sort((a, b) => {
        const dateA = a.updated_at ? new Date(a.updated_at).getTime() : 0;
        const dateB = b.updated_at ? new Date(b.updated_at).getTime() : 0;
        return dateB - dateA;
      });
    case "name":
      return list.sort((a, b) => a.name.localeCompare(b.name));
    default:
      return list;
  }
}
