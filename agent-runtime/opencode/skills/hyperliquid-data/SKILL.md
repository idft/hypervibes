# Hyperliquid Data

Fetch OHLCV candle data directly from Hyperliquid's public REST API.

## Usage

Run the script from the agent workspace root:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> [--limit N] [--start-time EPOCH_MS] [--end-time EPOCH_MS]
```

Examples:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m --limit 100
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py ETH 1h --start-time 1710000000000 --end-time 1710100000000
```

## Environment

Set `HYPERLIQUID_ENVIRONMENT` to `mainnet` or `testnet`. Defaults to `mainnet`.

## Supported timeframes

`5m`, `15m`, `1h`, `4h`, `1d` (and any others Hyperliquid supports).

## Output

Pretty-printed JSON array of candles to stdout. Each candle is returned in Hyperliquid's native order:

`[open, close, high, low, volume, timestamp_ms]`

Errors are printed to stderr and the script exits non-zero.
