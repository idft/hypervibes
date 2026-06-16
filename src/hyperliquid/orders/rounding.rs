//! Price and size rounding for Hyperliquid orders.
//!
//! The exchange requires every `limit_px` and `sz` to be aligned to the
//! instrument's tick and lot size. This module provides pure functions the
//! gateway uses to do that rounding server-side, conservatively, before
//! signing.

use rust_decimal::{Decimal, RoundingStrategy};

/// Default slippage tolerance for market orders, in basis points.
///
/// 50 bps = 0.5%. The gateway's market-order implementation widens the
/// mid price by this fraction to compute a worst-acceptable `limit_px`
/// for a `FrontendMarket` order.
pub const DEFAULT_MARKET_SLIPPAGE_BPS: u32 = 50;

/// Round an order size DOWN to the instrument's `size_decimals`.
///
/// Rounding down ensures the rounded size never exceeds the caller's
/// requested size.
pub fn round_size(size: Decimal, size_decimals: u32) -> Decimal {
    if size.is_sign_negative() {
        // Size is never negative in this flow. If it somehow is, we still
        // round toward zero (truncate) so the result fits in the lot.
        return size.round_dp_with_strategy(size_decimals, RoundingStrategy::ToZero);
    }
    size.round_dp_with_strategy(size_decimals, RoundingStrategy::ToZero)
}

/// Round an order price to the instrument's `price_decimals` and
/// Hyperliquid's 5 significant-figure price limit.
///
/// Conservative direction:
/// - **buy** limits round DOWN — a buyer's cap should never be higher
///   than they asked for.
/// - **sell** limits round UP — a seller's floor should never be lower
///   than they asked for.
///
/// This makes the rounded limit "less aggressive" than the requested
/// limit, which is the safe direction.
pub fn round_price(price: Decimal, price_decimals: u32, is_buy: bool) -> Decimal {
    let strategy = if is_buy {
        RoundingStrategy::ToNegativeInfinity
    } else {
        RoundingStrategy::ToPositiveInfinity
    };
    let decimals = effective_price_decimals(price, price_decimals);
    price.round_dp_with_strategy(decimals, strategy)
}

fn effective_price_decimals(price: Decimal, price_decimals: u32) -> u32 {
    let abs = price.abs();
    if abs.is_zero() {
        return price_decimals;
    }

    if abs >= Decimal::ONE {
        let integer_digits = integer_digits(abs);
        return price_decimals.min(5u32.saturating_sub(integer_digits));
    }

    let mut shifted = abs;
    let mut shifts = 0u32;
    while shifted < Decimal::ONE {
        shifted *= Decimal::from(10u32);
        shifts += 1;
        if shifts > price_decimals.saturating_add(5) {
            break;
        }
    }

    price_decimals.min(shifts + 4)
}

fn integer_digits(price: Decimal) -> u32 {
    let mut whole = price.round_dp_with_strategy(0, RoundingStrategy::ToZero);
    let mut digits = 0u32;
    while whole >= Decimal::ONE {
        whole /= Decimal::from(10u32);
        digits += 1;
    }
    digits
}

/// Compute a worst-acceptable limit price for a market (FrontendMarket)
/// order. Buys widen the mid UP by `slippage_bps`; sells widen DOWN.
///
/// The result is then rounded with [`round_price`].
pub fn market_guard_price(
    mid: Decimal,
    is_buy: bool,
    slippage_bps: u32,
    price_decimals: u32,
) -> Decimal {
    let slippage = Decimal::from(slippage_bps) / Decimal::from(10_000u32);
    let one = Decimal::from(1u32);
    let widened = if is_buy {
        mid * (one + slippage)
    } else {
        mid * (one - slippage)
    };
    round_price(widened, price_decimals, is_buy)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn round_size_truncates_to_decimals() {
        assert_eq!(round_size(dec!(0.123456), 5), dec!(0.12345));
        assert_eq!(round_size(dec!(0.123456), 4), dec!(0.1234));
        assert_eq!(round_size(dec!(0.123456), 6), dec!(0.123456));
    }

    #[test]
    fn round_size_never_increases() {
        let s = dec!(0.99999);
        let r = round_size(s, 2);
        assert!(r <= s, "rounded size {r} must be <= requested {s}");
    }

    #[test]
    fn round_size_handles_zero() {
        assert_eq!(round_size(dec!(0), 5), dec!(0));
    }

    #[test]
    fn round_size_handles_more_decimals_than_input() {
        // Request has 2 decimals; rounding to 5 must be a no-op.
        assert_eq!(round_size(dec!(1.23), 5), dec!(1.23));
    }

    #[test]
    fn round_price_buy_rounds_down() {
        assert_eq!(round_price(dec!(50000.567), 1, true), dec!(50000));
    }

    #[test]
    fn round_price_sell_rounds_up() {
        assert_eq!(round_price(dec!(50000.567), 1, false), dec!(50001));
    }

    #[test]
    fn round_price_keeps_decimal_when_within_sig_figs() {
        assert_eq!(round_price(dec!(9999.987), 1, true), dec!(9999.9));
        assert_eq!(round_price(dec!(9999.987), 1, false), dec!(10000.0));
    }

    #[test]
    fn round_price_allows_more_decimals_for_sub_one_prices() {
        assert_eq!(round_price(dec!(0.01234567), 8, true), dec!(0.012345));
        assert_eq!(round_price(dec!(0.01234567), 8, false), dec!(0.012346));
    }

    #[test]
    fn round_price_handles_negative_buy_and_sell() {
        // Direction is by sign of `is_buy`; negative prices round with
        // the chosen strategy as well.
        assert_eq!(round_price(dec!(-1.234), 2, true), dec!(-1.24));
        assert_eq!(round_price(dec!(-1.234), 2, false), dec!(-1.23));
    }

    #[test]
    fn round_price_handles_zero_and_exact() {
        assert_eq!(round_price(dec!(0), 3, true), dec!(0));
        assert_eq!(round_price(dec!(42.123), 3, true), dec!(42.123));
    }

    #[test]
    fn market_guard_price_buy_widens_up() {
        let p = market_guard_price(dec!(50000), true, 50, 1);
        assert_eq!(p, dec!(50250));
    }

    #[test]
    fn market_guard_price_sell_widens_down() {
        let p = market_guard_price(dec!(50000), false, 50, 1);
        assert_eq!(p, dec!(49750));
    }

    #[test]
    fn market_guard_price_custom_slippage() {
        // 100 bps = 1.0% — buy mid 100 -> 101.0
        let p = market_guard_price(dec!(100), true, 100, 2);
        assert_eq!(p, dec!(101.00));
        // sell -> 99.0
        let p = market_guard_price(dec!(100), false, 100, 2);
        assert_eq!(p, dec!(99.00));
    }

    #[test]
    fn market_guard_price_handles_zero_mid() {
        let p = market_guard_price(dec!(0), true, 50, 1);
        assert_eq!(p, dec!(0.0));
    }

    #[test]
    fn market_guard_price_handles_zero_decimals() {
        // 50 * 1.005 = 50.25; buy rounds DOWN to 0dp = 50.
        let p = market_guard_price(dec!(50), true, 50, 0);
        assert_eq!(p, dec!(50));
        // 50 * 0.995 = 49.75; sell rounds UP to 0dp = 50.
        let p = market_guard_price(dec!(50), false, 50, 0);
        assert_eq!(p, dec!(50));
    }

    #[test]
    fn default_slippage_constant_is_50bps() {
        assert_eq!(DEFAULT_MARKET_SLIPPAGE_BPS, 50);
    }
}
