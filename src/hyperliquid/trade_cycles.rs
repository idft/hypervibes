//! Deterministic, exchange-P&L-based flat-to-flat perpetual trade projection.
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CycleFill {
    pub hash: String,
    pub trade_id: String,
    pub fill_time: DateTime<Utc>,
    pub side: String,
    pub size: Decimal,
    pub price: Decimal,
    pub start_position: Option<Decimal>,
    pub fee: Option<Decimal>,
    pub fee_token: Option<String>,
    pub realized_pnl_usdc: Option<Decimal>,
}

#[derive(Debug, Clone)]
pub struct FillPart {
    pub hash: String,
    pub trade_id: String,
    pub segment: &'static str,
    pub quantity: Decimal,
    pub fee_usdc: Option<Decimal>,
    pub realized_pnl_usdc: Decimal,
}

#[derive(Debug, Clone)]
pub struct TradeCycle {
    pub id: Uuid,
    pub opening_hash: String,
    pub opening_trade_id: String,
    pub opening_segment: &'static str,
    pub direction: &'static str,
    pub status: &'static str,
    pub opened_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub remaining_size: Decimal,
    pub entry_size: Decimal,
    pub exit_size: Decimal,
    pub entry_notional: Decimal,
    pub exit_notional: Decimal,
    pub gross_pnl: Decimal,
    pub usdc_fees: Decimal,
    pub net_pnl: Decimal,
    pub fee_coverage: bool,
    pub coverage_reason: Option<String>,
    pub parts: Vec<FillPart>,
}

fn new_cycle(
    account: &str,
    environment: &str,
    instrument: &str,
    fill: &CycleFill,
    segment: &'static str,
    direction: &'static str,
    incomplete: bool,
) -> TradeCycle {
    let key = format!(
        "{}|{environment}|{instrument}|{direction}|{}|{}|{segment}",
        account.to_ascii_lowercase(),
        fill.hash,
        fill.trade_id
    );
    TradeCycle {
        id: Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes()),
        opening_hash: fill.hash.clone(),
        opening_trade_id: fill.trade_id.clone(),
        opening_segment: segment,
        direction,
        status: if incomplete { "incomplete" } else { "open" },
        opened_at: fill.fill_time,
        last_activity_at: fill.fill_time,
        closed_at: None,
        remaining_size: Decimal::ZERO,
        entry_size: Decimal::ZERO,
        exit_size: Decimal::ZERO,
        entry_notional: Decimal::ZERO,
        exit_notional: Decimal::ZERO,
        gross_pnl: Decimal::ZERO,
        usdc_fees: Decimal::ZERO,
        net_pnl: Decimal::ZERO,
        fee_coverage: true,
        coverage_reason: incomplete
            .then(|| "Position predates or disagrees with available fills".into()),
        parts: Vec::new(),
    }
}

fn add_part(
    cycle: &mut TradeCycle,
    fill: &CycleFill,
    segment: &'static str,
    quantity: Decimal,
    fee: Option<Decimal>,
    pnl: Decimal,
) {
    if segment == "exit" {
        cycle.exit_size += quantity;
        cycle.exit_notional += quantity * fill.price;
    } else {
        cycle.entry_size += quantity;
        cycle.entry_notional += quantity * fill.price;
    }
    cycle.last_activity_at = fill.fill_time;
    cycle.gross_pnl += pnl;
    if let Some(fee) = fee {
        cycle.usdc_fees += fee;
    } else {
        cycle.fee_coverage = false;
        cycle
            .coverage_reason
            .get_or_insert_with(|| "Fee is missing or not denominated in USDC".into());
    }
    cycle.net_pnl = cycle.gross_pnl - cycle.usdc_fees;
    cycle.parts.push(FillPart {
        hash: fill.hash.clone(),
        trade_id: fill.trade_id.clone(),
        segment,
        quantity,
        fee_usdc: fee,
        realized_pnl_usdc: pnl,
    });
}

/// Inputs must be from one exchange account and canonical perpetual instrument.
/// No rounding is applied when a flip divides fees between the two cycles.
pub fn build_cycles(
    account: &str,
    environment: &str,
    instrument: &str,
    fills: &mut [CycleFill],
) -> Result<Vec<TradeCycle>> {
    fills.sort_by(|a, b| {
        a.fill_time
            .cmp(&b.fill_time)
            .then_with(|| {
                match (
                    a.trade_id.trim_matches('"').parse::<u128>(),
                    b.trade_id.trim_matches('"').parse::<u128>(),
                ) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => a.trade_id.cmp(&b.trade_id),
                }
            })
            .then_with(|| a.hash.cmp(&b.hash))
    });
    let mut cycles = Vec::new();
    let mut active: Option<TradeCycle> = None;
    let mut signed = Decimal::ZERO;
    let mut group_end = 0;
    for index in 0..fills.len() {
        if index == group_end {
            group_end = index + 1;
            while group_end < fills.len() && fills[group_end].fill_time == fills[index].fill_time {
                group_end += 1;
            }
        }
        // Exchange fill IDs are identities, not execution sequence numbers.
        // For timestamp ties, follow the reported position chain first; keep
        // the deterministic ID/hash order when no matching position is known.
        if let Some(next) = fills[index..group_end]
            .iter()
            .position(|fill| fill.start_position == Some(signed))
        {
            fills[index..=index + next].rotate_right(1);
        }
        let fill = &fills[index];
        ensure!(fill.size > Decimal::ZERO, "non-positive fill size");
        let sign = match fill.side.as_str() {
            "B" | "buy" => Decimal::ONE,
            "A" | "sell" => -Decimal::ONE,
            _ => {
                if let Some(mut cycle) = active.take() {
                    cycle.status = "incomplete";
                    cycle.coverage_reason = Some("Unrecognized exchange side".into());
                    cycles.push(cycle);
                }
                let mut unknown = new_cycle(
                    account,
                    environment,
                    instrument,
                    fill,
                    "entry",
                    "unknown",
                    true,
                );
                unknown.coverage_reason = Some("Unrecognized exchange side".into());
                unknown.remaining_size = fill.size;
                add_part(&mut unknown, fill, "entry", fill.size, None, Decimal::ZERO);
                cycles.push(unknown);
                signed = Decimal::ZERO;
                continue;
            }
        };
        if let Some(start) = fill.start_position {
            if start != signed {
                if let Some(mut cycle) = active.take() {
                    cycle.status = "incomplete";
                    cycle.coverage_reason =
                        Some("Exchange startPosition disagrees with reconstructed size".into());
                    cycles.push(cycle);
                }
                signed = start;
                if !signed.is_zero() {
                    active = Some(new_cycle(
                        account,
                        environment,
                        instrument,
                        fill,
                        "entry",
                        if signed.is_sign_positive() {
                            "long"
                        } else {
                            "short"
                        },
                        true,
                    ));
                    if let Some(cycle) = active.as_mut() {
                        cycle.remaining_size = signed.abs();
                    }
                }
            }
        } else if let Some(cycle) = active.as_mut() {
            cycle.status = "incomplete";
            cycle.coverage_reason = Some("Exchange startPosition missing".into());
        }
        let closing = if !signed.is_zero() && signed.is_sign_positive() != sign.is_sign_positive() {
            signed.abs().min(fill.size)
        } else {
            Decimal::ZERO
        };
        let opening = fill.size - closing;
        let fee = if fill
            .fee_token
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("USDC"))
        {
            fill.fee
        } else {
            None
        };
        let pnl = fill.realized_pnl_usdc.unwrap_or_default();
        if closing > Decimal::ZERO {
            let cycle = active
                .as_mut()
                .expect("nonzero reconstructed position has an active cycle");
            if fill.realized_pnl_usdc.is_none() {
                cycle.status = "incomplete";
                cycle.coverage_reason = Some("Exchange closing P&L missing".into());
            }
            // Closing P&L belongs entirely to the closing portion of a flip.
            let closing_fee = fee.map(|value| value * closing / fill.size);
            add_part(cycle, fill, "exit", closing, closing_fee, pnl);
            signed += sign * closing;
            cycle.remaining_size = signed.abs();
            if signed.is_zero() {
                cycle.closed_at = Some(fill.fill_time);
                if cycle.status == "open" {
                    cycle.status = "closed";
                }
                cycles.push(active.take().expect("closing cycle exists"));
            }
        }
        if opening > Decimal::ZERO {
            let segment = if closing > Decimal::ZERO {
                "flip_entry"
            } else {
                "entry"
            };
            if active.is_none() {
                active = Some(new_cycle(
                    account,
                    environment,
                    instrument,
                    fill,
                    segment,
                    if sign.is_sign_positive() {
                        "long"
                    } else {
                        "short"
                    },
                    fill.start_position.is_none(),
                ));
            }
            signed += sign * opening;
            let cycle = active.as_mut().expect("opening cycle exists");
            cycle.remaining_size = signed.abs();
            let opening_fee = fee.map(|value| value - value * closing / fill.size);
            add_part(cycle, fill, segment, opening, opening_fee, Decimal::ZERO);
        }
    }
    if let Some(cycle) = active {
        cycles.push(cycle);
    }
    Ok(cycles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    fn fill(
        id: &str,
        side: &str,
        size: Decimal,
        start: Decimal,
        fee: Decimal,
        pnl: Decimal,
    ) -> CycleFill {
        CycleFill {
            hash: id.into(),
            trade_id: id.into(),
            fill_time: Utc::now(),
            side: side.into(),
            size,
            price: dec!(100),
            start_position: Some(start),
            fee: Some(fee),
            fee_token: Some("USDC".into()),
            realized_pnl_usdc: Some(pnl),
        }
    }
    #[test]
    fn flip_allocates_fee_and_preserves_stable_ids() {
        let mut fills = vec![
            fill("1", "B", dec!(2), dec!(0), dec!(2), dec!(0)),
            fill("2", "A", dec!(3), dec!(2), dec!(3), dec!(10)),
        ];
        let cycles = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].status, "closed");
        assert_eq!(cycles[0].gross_pnl, dec!(10));
        assert_eq!(cycles[0].usdc_fees, dec!(4));
        assert_eq!(cycles[1].direction, "short");
        assert_eq!(cycles[1].usdc_fees, dec!(1));
        assert_eq!(cycles[1].remaining_size, dec!(1));
        let again = build_cycles("account", "live", "BTC", &mut fills).expect("rebuild");
        assert_eq!(cycles[0].id, again[0].id);
    }

    #[test]
    fn partial_exit_and_short_reentry_are_separate_cycles() {
        let mut fills = vec![
            fill("1", "buy", dec!(1), dec!(0), dec!(0.1), dec!(0)),
            fill("2", "B", dec!(1), dec!(1), dec!(0.1), dec!(0)),
            fill("3", "sell", dec!(0.5), dec!(2), dec!(0.05), dec!(2)),
            fill("4", "A", dec!(1.5), dec!(1.5), dec!(0.15), dec!(5)),
            fill("5", "sell", dec!(1), dec!(0), dec!(0.1), dec!(0)),
            fill("6", "buy", dec!(1), dec!(-1), dec!(0.1), dec!(3)),
        ];
        for (index, fill) in fills.iter_mut().enumerate() {
            fill.fill_time += chrono::Duration::seconds(index as i64);
        }
        let result = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].status, "closed");
        assert_eq!(result[0].entry_size, dec!(2));
        assert_eq!(result[0].exit_size, dec!(2));
        assert_eq!(result[0].gross_pnl, dec!(7));
        assert_eq!(result[1].direction, "short");
        assert_eq!(result[1].status, "closed");
        assert_eq!(result[1].gross_pnl, dec!(3));
    }

    #[test]
    fn initial_position_or_unknown_side_is_not_a_complete_trade() {
        let mut fills = vec![fill("first", "sell", dec!(1), dec!(2), dec!(1), dec!(5))];
        let result = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert_eq!(result[0].status, "incomplete");
        assert_eq!(result[0].entry_size, dec!(0));
        fills = vec![fill("unknown", "?", dec!(1), dec!(0), dec!(1), dec!(0))];
        let result = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert_eq!(result[0].status, "incomplete");
        assert_eq!(result[0].direction, "unknown");
    }

    #[test]
    fn same_timestamp_entry_precedes_its_close() {
        let mut fills = vec![
            fill("10", "sell", dec!(1), dec!(1), dec!(0), dec!(3)),
            fill("2", "buy", dec!(1), dec!(0), dec!(0), dec!(0)),
        ];
        fills[1].fill_time = fills[0].fill_time;
        let result = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, "closed");
    }

    #[test]
    fn timestamp_ties_follow_positions_instead_of_fill_ids() {
        let mut fills = vec![
            fill(
                "1085844328340608",
                "sell",
                dec!(0.0002),
                dec!(0),
                dec!(0.01),
                dec!(0),
            ),
            fill(
                "609855250121526",
                "sell",
                dec!(0.0002),
                dec!(-0.0002),
                dec!(0.01),
                dec!(0),
            ),
            fill(
                "272716312003457",
                "buy",
                dec!(0.0002),
                dec!(-0.0002),
                dec!(0.01),
                dec!(-0.0644),
            ),
            fill(
                "980106919270609",
                "buy",
                dec!(0.0002),
                dec!(-0.0004),
                dec!(0.01),
                dec!(-0.0644),
            ),
        ];
        let time = fills[0].fill_time;
        fills[1].fill_time = time + chrono::Duration::minutes(1);
        fills[2].fill_time = time + chrono::Duration::minutes(2);
        fills[3].fill_time = fills[2].fill_time;
        let result = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert_eq!(result.len(), 1);
        let cycle = &result[0];
        assert_eq!(cycle.status, "closed");
        assert_eq!(cycle.direction, "short");
        assert_eq!(cycle.coverage_reason, None);
        assert_eq!(cycle.entry_size, dec!(0.0004));
        assert_eq!(cycle.exit_size, cycle.entry_size);
        assert_eq!(cycle.remaining_size, Decimal::ZERO);
        assert_eq!(cycle.gross_pnl, dec!(-0.1288));
        assert_eq!(cycle.usdc_fees, dec!(0.04));
        assert_eq!(cycle.parts[2].trade_id, "980106919270609");
        fills.reverse();
        let again = build_cycles("account", "live", "BTC", &mut fills).expect("rebuild");
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].id, cycle.id);
    }

    #[test]
    fn timestamp_ties_do_not_hide_real_position_gaps() {
        let mut fills = vec![
            fill("1", "buy", dec!(1), dec!(0), dec!(0), dec!(0)),
            fill("2", "sell", dec!(1), dec!(3), dec!(0), dec!(2)),
            fill("3", "sell", dec!(1), dec!(2), dec!(0), dec!(2)),
        ];
        fills[1].fill_time = fills[0].fill_time + chrono::Duration::seconds(1);
        fills[2].fill_time = fills[1].fill_time;
        let result = build_cycles("account", "live", "BTC", &mut fills).expect("build");
        assert!(result.iter().all(|cycle| cycle.status == "incomplete"));
    }
}
