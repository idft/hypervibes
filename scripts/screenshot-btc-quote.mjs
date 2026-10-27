export const btcQuoteUrl = "https://api.exchange.coinbase.com/products/BTC-USD/ticker";
export const btcQuoteTimeoutMs = 10000;
// Reject old ticker trades and implausible future times. Never reuse a quote.
export const btcQuoteMaxAgeMs = 60000;
export const btcQuoteFutureToleranceMs = 5000;

export function validateBtcQuote(data, now = Date.now()) {
  // Preserve the decimal string. Do not use floating-point arithmetic for prices.
  if (!data || typeof data.price !== "string"
    || !/^(?:0|[1-9]\d{0,15})(?:\.\d{1,8})?$/.test(data.price)
    || BigInt(data.price.replace(".", "")) <= 0n) {
    throw new Error("Coinbase BTC-USD ticker has an invalid price");
  }
  const [whole, fraction = ""] = data.price.split(".");
  const scaledPrice = BigInt(whole) * 100000000n + BigInt(fraction.padEnd(8, "0"));
  if (scaledPrice < 1000n * 100000000n || scaledPrice > 1000000n * 100000000n) {
    throw new Error("Coinbase BTC-USD ticker has an invalid price for the screenshot (supported range: 1000 to 1000000 USD)");
  }
  const timestamp = data.time;
  const milliseconds = typeof timestamp === "string" ? Date.parse(timestamp) : NaN;
  if (typeof timestamp !== "string"
    || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/.test(timestamp)
    || !Number.isFinite(milliseconds)
    || new Date(milliseconds).toISOString().slice(0, 19) !== timestamp.slice(0, 19)) {
    throw new Error("Coinbase BTC-USD ticker has an invalid timestamp");
  }
  if (now - milliseconds > btcQuoteMaxAgeMs || milliseconds - now > btcQuoteFutureToleranceMs) {
    throw new Error("Coinbase BTC-USD ticker timestamp is not fresh");
  }
  return { price: data.price, source: btcQuoteUrl, timestamp };
}

export async function fetchBtcQuote({ signal, fetch: fetchRequest = globalThis.fetch, now = Date.now } = {}) {
  signal?.throwIfAborted();
  const deadline = new AbortController();
  const requestSignal = signal ? AbortSignal.any([signal, deadline.signal]) : deadline.signal;
  const timer = setTimeout(() => deadline.abort(new Error("Coinbase BTC-USD quote timed out after 10 seconds")), btcQuoteTimeoutMs);
  let onAbort;
  // The race also bounds response-body reads and injected fetch implementations.
  const aborted = new Promise((_resolve, reject) => {
    onAbort = () => reject(requestSignal.reason);
    requestSignal.addEventListener("abort", onAbort, { once: true });
  });
  try {
    return await Promise.race([
      aborted,
      (async () => {
        const response = await fetchRequest(btcQuoteUrl, {
          method: "GET",
          headers: { Accept: "application/json" },
          credentials: "omit",
          cache: "no-store",
          redirect: "error",
          signal: requestSignal,
        });
        requestSignal.throwIfAborted();
        if (!response.ok) {
          await response.body?.cancel();
          throw new Error(`Coinbase BTC-USD ticker returned HTTP ${response.status}`);
        }
        const data = await response.json();
        requestSignal.throwIfAborted();
        return validateBtcQuote(data, now());
      })(),
    ]);
  } finally {
    clearTimeout(timer);
    requestSignal.removeEventListener("abort", onAbort);
  }
}
