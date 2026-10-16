/** Exact finite-decimal accounting. Division alone rounds half-even at 28 places.
 * Split fees use subtraction for the remainder, preserving the source total. */
export class Amount {
  private constructor(readonly units: bigint, readonly scale: number) {}
  static from(value: string): Amount {
    if (value.length > 200 || !/^-?\d+(?:\.\d+)?$/.test(value)) throw new Error("Invalid journal decimal");
    const [whole, fraction = ""] = value.split(".");
    return new Amount(BigInt(`${whole}${fraction}`), fraction.length);
  }
  private align(other: Amount) {
    const scale = Math.max(this.scale, other.scale);
    return { a: this.units * 10n ** BigInt(scale - this.scale), b: other.units * 10n ** BigInt(scale - other.scale), scale };
  }
  add(other: Amount) { const { a, b, scale } = this.align(other); return new Amount(a + b, scale); }
  sub(other: Amount) { return this.add(other.neg()); }
  neg() { return new Amount(-this.units, this.scale); }
  abs() { return this.units < 0n ? this.neg() : this; }
  mul(other: Amount) { return new Amount(this.units * other.units, this.scale + other.scale); }
  div(other: Amount) {
    if (other.units === 0n) throw new Error("Division by zero");
    const numerator = this.units * 10n ** BigInt(other.scale + 28);
    const denominator = other.units * 10n ** BigInt(this.scale);
    const negative = (numerator < 0n) !== (denominator < 0n);
    const n = numerator < 0n ? -numerator : numerator;
    const d = denominator < 0n ? -denominator : denominator;
    let result = n / d;
    const remainder = n % d;
    if (remainder * 2n > d || (remainder * 2n === d && result % 2n !== 0n)) result++;
    return new Amount(negative ? -result : result, 28);
  }
  cmp(other: Amount) { const { a, b } = this.align(other); return a < b ? -1 : a > b ? 1 : 0; }
  get zero() { return this.units === 0n; }
  get sign() { return this.units < 0n ? -1 : this.units > 0n ? 1 : 0; }
  toString() {
    const raw = (this.units < 0n ? -this.units : this.units).toString().padStart(this.scale + 1, "0");
    const body = this.scale ? `${raw.slice(0, -this.scale)}.${raw.slice(-this.scale)}`.replace(/\.?0+$/, "") : raw;
    return `${this.units < 0n ? "-" : ""}${body}`;
  }
}
export const amount = Amount.from;
