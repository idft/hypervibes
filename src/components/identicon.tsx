/**
 * Deterministic 8×8 identicon for a wallet address, as an SVG data URI.
 * Port of the legacy application's `identicon_data_uri` so existing users keep
 * the same identicon for their address.
 */

function identiconDataUri(address: string): string {
  let seed = 0;
  for (let index = 0; index < address.length; index++) {
    seed = (seed * 32 - seed + address.charCodeAt(index)) >>> 0;
  }
  const random = () => {
    seed = (seed * 9_301 + 49_297) % 233_280;
    return seed / 233_280;
  };
  const color = () =>
    `hsl(${Math.floor(random() * 360)} ${Math.floor(random() * 60 + 40)}% ${Math.floor((random() + random() + random() + random()) * 25)}%)`;
  const foreground = color();
  const background = color();
  const spot = color();
  let squares = "";
  for (let row = 0; row < 8; row++) {
    const values = [0, 1, 2, 3].map(() => Math.floor(random() * 2.3));
    values.push(values[3] ?? 0, values[2] ?? 0, values[1] ?? 0, values[0] ?? 0);
    for (const [column, value] of values.entries()) {
      if (value !== 0) {
        squares += `<rect x="${column}" y="${row}" width="1" height="1" fill="${value === 1 ? foreground : spot}"/>`;
      }
    }
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="${background}"/>${squares}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function Identicon({ address, className }: { address: string; className?: string }) {
  return <img src={identiconDataUri(address)} alt="" className={className} />;
}