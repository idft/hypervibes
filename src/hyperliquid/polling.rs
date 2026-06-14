use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollingSchedule {
    pub fills: Duration,
    pub funding: Duration,
    pub ledger: Duration,
    pub historical_orders: Duration,
}

impl Default for PollingSchedule {
    fn default() -> Self {
        Self {
            fills: Duration::from_secs(60),
            funding: Duration::from_secs(300),
            ledger: Duration::from_secs(300),
            historical_orders: Duration::from_secs(900),
        }
    }
}
