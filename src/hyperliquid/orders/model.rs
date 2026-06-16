//! JSON DTOs for the `/api/v1/orders` endpoints and request validation.
//!
//! These types describe the wire contract for the execution gateway. All
//! numeric values are [`rust_decimal::Decimal`] (not `f64`).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---- request types --------------------------------------------------------

/// Top-level `POST /api/v1/orders` body.
#[derive(Debug, Deserialize)]
pub struct PlaceOrdersRequest {
    pub orders: Vec<PlaceOrderInput>,
}

/// A single order to place. One entry + its TP/SL legs form one logical
/// group; the gateway still records each leg as its own row in
/// `hyperliquid.orders`, sharing the same `group_id`.
#[derive(Debug, Deserialize)]
pub struct PlaceOrderInput {
    /// Symbol the agent sends, e.g. `"BTC"`. Resolved against
    /// `hyperliquid.instruments.instrument_id`.
    pub symbol: String,
    /// `"buy"` or `"sell"`.
    pub side: String,
    /// `"limit"` or `"market"`.
    pub order_type: String,
    /// Size in base asset units. Must be > 0.
    pub size: Decimal,
    /// Required for limit orders. Ignored for market orders.
    pub price: Option<Decimal>,
    /// Time-in-force. `"gtc"` (default for limit), `"ioc"`, or `"alo"`.
    /// Ignored for market orders (the gateway uses
    /// [`hypersdk::hypercore::types::TimeInForce::FrontendMarket`]).
    #[serde(default)]
    pub time_in_force: Option<String>,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub take_profits: Vec<TriggerInput>,
    #[serde(default)]
    pub stop_losses: Vec<TriggerInput>,
    /// Optional IDs of memory records that justified the decision. Carried
    /// through onto the `hyperliquid.orders` row.
    #[serde(default)]
    pub memory_record_ids: Vec<String>,
}

/// A take-profit or stop-loss leg attached to an entry order.
#[derive(Debug, Deserialize)]
pub struct TriggerInput {
    /// Trigger price for the TP or SL.
    pub trigger_price: Decimal,
    /// Limit price used when the trigger fires. If `None`, the gateway
    /// treats the leg as a market-on-trigger (SL is always market-on-trigger
    /// for v1, regardless of this field).
    pub limit_price: Option<Decimal>,
    /// Size for the leg. If `None`, the gateway uses the entry size.
    pub size: Option<Decimal>,
}

/// `POST /api/v1/orders/cancel` body.
#[derive(Debug, Deserialize)]
pub struct CancelOrdersRequest {
    pub orders: Vec<CancelInput>,
}

/// One cancellation. The `oid` is the exchange-assigned order ID, which
/// the agent gets from `GET /api/v1/orders`.
#[derive(Debug, Deserialize)]
pub struct CancelInput {
    pub symbol: String,
    pub oid: u64,
}

// ---- response types -------------------------------------------------------

/// Top-level `POST /api/v1/orders` response.
#[derive(Debug, Serialize)]
pub struct PlaceOrdersResponse {
    pub results: Vec<OrderResult>,
}

/// Per-leg outcome of a place request. The order matches the input list
/// (entry + TP legs + SL legs in that order, per group).
#[derive(Debug, Serialize)]
pub struct OrderResult {
    pub id: Uuid,
    pub cloid: String,
    pub symbol: String,
    pub side: String,
    /// `"limit" | "market" | "take_profit" | "stop_loss"`.
    pub order_kind: String,
    pub status: String,
    pub exchange_oid: Option<String>,
    pub group_id: Option<Uuid>,
    /// Per-leg error string; `null` when the leg was accepted.
    pub error: Option<String>,
}

// ---- validation -----------------------------------------------------------

impl PlaceOrderInput {
    /// Validate this single order input. Returns the first error message,
    /// or `Ok(())`. Used by [`PlaceOrdersRequest::validate`].
    pub fn validate(&self) -> Result<(), String> {
        match self.side.as_str() {
            "buy" | "sell" => {}
            other => return Err(format!("side must be 'buy' or 'sell', got '{other}'")),
        }

        match self.order_type.as_str() {
            "limit" | "market" => {}
            other => {
                return Err(format!(
                    "order_type must be 'limit' or 'market', got '{other}'"
                ));
            }
        }

        if self.size <= Decimal::ZERO {
            return Err("size must be > 0".to_string());
        }

        if self.order_type == "limit" {
            match self.price {
                Some(p) if p > Decimal::ZERO => {}
                Some(_) => return Err("price must be > 0 for limit orders".to_string()),
                None => return Err("price is required for limit orders".to_string()),
            }
        }

        for (idx, tp) in self.take_profits.iter().enumerate() {
            tp.validate()
                .map_err(|e| format!("take_profits[{idx}]: {e}"))?;
        }
        for (idx, sl) in self.stop_losses.iter().enumerate() {
            sl.validate()
                .map_err(|e| format!("stop_losses[{idx}]: {e}"))?;
        }

        Ok(())
    }
}

impl TriggerInput {
    fn validate(&self) -> Result<(), String> {
        if self.trigger_price <= Decimal::ZERO {
            return Err("trigger_price must be > 0".to_string());
        }
        if let Some(limit) = self.limit_price
            && limit <= Decimal::ZERO
        {
            return Err("limit_price must be > 0 when set".to_string());
        }
        if let Some(size) = self.size
            && size <= Decimal::ZERO
        {
            return Err("size must be > 0 when set".to_string());
        }
        Ok(())
    }
}

impl PlaceOrdersRequest {
    /// Validate the request. Errors are joined into a single string with
    /// the offending input index.
    pub fn validate(&self) -> Result<(), String> {
        if self.orders.is_empty() {
            return Err("orders must not be empty".to_string());
        }
        for (idx, order) in self.orders.iter().enumerate() {
            if order.symbol.trim().is_empty() {
                return Err(format!("orders[{idx}]: symbol is required"));
            }
            order
                .validate()
                .map_err(|e| format!("orders[{idx}]: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn good_limit() -> PlaceOrderInput {
        PlaceOrderInput {
            symbol: "BTC".to_string(),
            side: "buy".to_string(),
            order_type: "limit".to_string(),
            size: dec!(0.1),
            price: Some(dec!(50000)),
            time_in_force: Some("gtc".to_string()),
            reduce_only: false,
            take_profits: vec![],
            stop_losses: vec![],
            memory_record_ids: vec![],
        }
    }

    #[test]
    fn validate_accepts_well_formed_limit() {
        assert!(good_limit().validate().is_ok());
    }

    #[test]
    fn validate_accepts_well_formed_market() {
        let mut o = good_limit();
        o.order_type = "market".to_string();
        o.price = None;
        assert!(o.validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_side() {
        let mut o = good_limit();
        o.side = "long".to_string();
        let err = o.validate().unwrap_err();
        assert!(err.contains("side"));
    }

    #[test]
    fn validate_rejects_bad_order_type() {
        let mut o = good_limit();
        o.order_type = "stop".to_string();
        let err = o.validate().unwrap_err();
        assert!(err.contains("order_type"));
    }

    #[test]
    fn validate_rejects_zero_size() {
        let mut o = good_limit();
        o.size = dec!(0);
        assert!(o.validate().unwrap_err().contains("size"));
    }

    #[test]
    fn validate_rejects_negative_size() {
        let mut o = good_limit();
        o.size = dec!(-0.1);
        assert!(o.validate().unwrap_err().contains("size"));
    }

    #[test]
    fn validate_rejects_limit_without_price() {
        let mut o = good_limit();
        o.price = None;
        let err = o.validate().unwrap_err();
        assert!(err.contains("price"));
    }

    #[test]
    fn validate_rejects_limit_with_zero_price() {
        let mut o = good_limit();
        o.price = Some(dec!(0));
        let err = o.validate().unwrap_err();
        assert!(err.contains("price"));
    }

    #[test]
    fn validate_rejects_bad_trigger_price() {
        let mut o = good_limit();
        o.take_profits = vec![TriggerInput {
            trigger_price: dec!(0),
            limit_price: None,
            size: None,
        }];
        let err = o.validate().unwrap_err();
        assert!(err.contains("take_profits[0]"));
    }

    #[test]
    fn validate_rejects_bad_trigger_size() {
        let mut o = good_limit();
        o.stop_losses = vec![TriggerInput {
            trigger_price: dec!(49000),
            limit_price: None,
            size: Some(dec!(-1)),
        }];
        let err = o.validate().unwrap_err();
        assert!(err.contains("stop_losses[0]"));
    }

    #[test]
    fn request_rejects_empty_orders() {
        let req = PlaceOrdersRequest { orders: vec![] };
        let err = req.validate().unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn request_rejects_empty_symbol() {
        let mut o = good_limit();
        o.symbol = "  ".to_string();
        let req = PlaceOrdersRequest { orders: vec![o] };
        let err = req.validate().unwrap_err();
        assert!(err.contains("symbol"));
    }

    #[test]
    fn request_includes_index_in_error() {
        let mut o1 = good_limit();
        o1.symbol = "BTC".to_string();
        let mut o2 = good_limit();
        o2.symbol = "ETH".to_string();
        o2.side = "long".to_string();
        let req = PlaceOrdersRequest {
            orders: vec![o1, o2],
        };
        let err = req.validate().unwrap_err();
        assert!(err.contains("orders[1]"), "got: {err}");
    }
}
