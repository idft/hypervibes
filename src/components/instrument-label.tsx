import { useState } from "react";
import { currencyLogoUrl } from "../hyperliquid/currency-logo.js";

/** Keeps the exchange symbol accessible, including when only the logo is visible. */
export function InstrumentLabel({ symbol, iconOnly = false }: { symbol: string; iconOnly?: boolean }) {
  const url = currencyLogoUrl(symbol);
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  return <span className="inline-flex items-center gap-2" title={iconOnly ? symbol : undefined}>
    {url && (failedUrl === url
      ? <span aria-hidden="true" className="flex h-5 w-5 shrink-0 items-center justify-center rounded-sm bg-zinc-800 text-[8px] text-zinc-300">{symbol.slice(0, 3)}</span>
      : <img src={url} alt="" aria-hidden="true" width={20} height={20} loading="lazy" onError={() => setFailedUrl(url)} className="h-5 w-5 shrink-0 rounded-sm bg-zinc-800" />)}
    <span className={iconOnly && url ? "sr-only" : undefined}>{symbol}</span>
  </span>;
}
