/** Provider ids are URL path segments into /model-catalog/logos and upstream models.dev. */
export function isProviderSlug(provider: string): boolean {
  return provider.length <= 64 && /^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(provider);
}

/** Provider ids that don't match their models.dev logo slug get remapped here. */
const logoSlugAliases: Readonly<Record<string, string>> = {
  // models.dev has no Codex logo; the OpenAI knot is the closest match.
  "openai-codex": "openai",
};

export function providerLogoSlug(provider: string): string | null {
  return isProviderSlug(provider) ? logoSlugAliases[provider] ?? provider : null;
}

export function providerLogoUrl(provider: string): string | null {
  const slug = providerLogoSlug(provider);
  return slug ? `/model-catalog/logos/${encodeURIComponent(slug)}.svg` : null;
}