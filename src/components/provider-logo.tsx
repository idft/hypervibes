import { providerLogoUrl } from "../providers/provider-logo-url.js";

export function ProviderLogo({ provider, className }: { provider: string; className?: string }) {
  const url = providerLogoUrl(provider);
  if (!url) return null;
  return <img src={url} alt={`${provider} logo`} loading="lazy" className={className} />;
}