import { useEffect, useState } from "react";
import { format } from "timeago.js";

export function ToggleTimestamp({ timestamp, label = "creation" }: { timestamp: number; label?: string }) {
  const [showUtc, setShowUtc] = useState(false);
  const [, tick] = useState(0);
  useEffect(() => {
    if (showUtc) return;
    const timer = setInterval(() => tick((value) => value + 1), 30000);
    return () => clearInterval(timer);
  }, [showUtc]);
  const exact = new Date(timestamp).toISOString();
  return <button type="button" className="cursor-pointer rounded text-left hover:text-white focus-visible:outline-violet-400" aria-label={showUtc ? `Show relative ${label} time` : `Show UTC ${label} timestamp`} title={showUtc ? "Click to show relative time" : `${exact} · Click to show UTC`} onClick={() => setShowUtc((value) => !value)}><time dateTime={exact} suppressHydrationWarning>{showUtc ? `${exact.replace("T", " ").slice(0, 19)} UTC` : format(timestamp)}</time></button>;
}
