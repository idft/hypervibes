use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Debug, Clone)]
pub struct MoneyCell {
    pub value: String,
    pub color_class: &'static str,
}

pub(super) fn format_decimal_with_commas(value: Decimal, decimals: usize) -> String {
    let formatted = format!("{:.precision$}", value, precision = decimals);
    add_thousands_separators(&formatted)
}

pub(super) fn add_thousands_separators(value: &str) -> String {
    let (sign, unsigned) = if let Some(stripped) = value.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = value.strip_prefix('+') {
        ("+", stripped)
    } else {
        ("", value)
    };

    let (integer, fractional) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let mut grouped_reversed = String::with_capacity(integer.len() + integer.len() / 3);
    for (index, ch) in integer.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped_reversed.push(',');
        }
        grouped_reversed.push(ch);
    }
    let grouped_integer: String = grouped_reversed.chars().rev().collect();

    if fractional.is_empty() {
        format!("{sign}{grouped_integer}")
    } else {
        format!("{sign}{grouped_integer}.{fractional}")
    }
}

pub(super) fn format_timestamp_utc(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M UTC").to_string()
}

pub(super) fn format_timestamp_iso(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

pub(super) fn format_optional_timestamp_utc(value: Option<DateTime<Utc>>) -> String {
    value
        .map(format_timestamp_utc)
        .unwrap_or_else(|| "-".to_string())
}

pub fn format_money_text(amount: Option<Decimal>) -> String {
    format_money_text_with_decimals(amount, 4)
}

pub fn format_money_text_with_decimals(amount: Option<Decimal>, decimals: usize) -> String {
    match amount {
        None => "-".to_string(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            if value.is_sign_negative() {
                format!("({formatted})")
            } else {
                formatted
            }
        }
    }
}

pub fn format_money_cell(amount: Option<Decimal>) -> MoneyCell {
    format_money_cell_with_decimals(amount, 4)
}

pub fn format_money_cell_with_decimals(amount: Option<Decimal>, decimals: usize) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            if value.is_sign_negative() {
                MoneyCell {
                    value: format!("({formatted})"),
                    color_class: "text-red-400",
                }
            } else {
                MoneyCell {
                    value: formatted,
                    color_class: "text-emerald-400",
                }
            }
        }
    }
}

/// Like [`format_money_cell`] but prefixes positive values with `+`.
/// Used for sparkline change amounts where the sign is part of the display.
pub fn format_signed_money_cell(amount: Option<Decimal>) -> MoneyCell {
    format_signed_money_cell_with_decimals(amount, 4)
}

pub fn format_signed_money_cell_with_decimals(
    amount: Option<Decimal>,
    decimals: usize,
) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            if value.is_sign_negative() {
                MoneyCell {
                    value: format!("({formatted})"),
                    color_class: "text-red-400",
                }
            } else {
                MoneyCell {
                    value: format!("+{formatted}"),
                    color_class: "text-emerald-400",
                }
            }
        }
    }
}

pub(super) fn dash_cell() -> MoneyCell {
    MoneyCell {
        value: "-".to_string(),
        color_class: "text-zinc-500",
    }
}

pub fn format_neutral_money_cell(amount: Option<Decimal>) -> MoneyCell {
    format_neutral_money_cell_with_decimals(amount, 4)
}

pub fn format_neutral_money_cell_with_decimals(amount: Option<Decimal>, decimals: usize) -> MoneyCell {
    match amount {
        None => dash_cell(),
        Some(value) if value.is_zero() => dash_cell(),
        Some(value) => {
            let formatted = format_decimal_with_commas(value.abs(), decimals);
            MoneyCell {
                value: if value.is_sign_negative() {
                    format!("({formatted})")
                } else {
                    formatted
                },
                color_class: "text-zinc-300",
            }
        }
    }
}

/// A number rendered as individual digit spans so the frontend can run a
/// roll animation when the value changes.
///
/// * `raw` is the numeric string used to decide animation direction.
/// * `value` is the full human-readable text shown to the user.
/// * `chars` are the characters that should actually roll; `prefix` and
///   `suffix` are rendered as static spans so symbols such as parentheses
///   around a negative PnL never animate or take on the flash color.
#[derive(Debug, Clone)]
pub struct AnimatedNumber {
    pub value: String,
    pub raw: String,
    pub chars: Vec<char>,
    pub color_class: &'static str,
    pub prefix: String,
    pub suffix: String,
}

impl AnimatedNumber {
    pub fn from_decimal(value: Decimal, color_class: &'static str) -> Self {
        let formatted = format_decimal_with_commas(value, 4);
        Self {
            raw: value.to_string(),
            value: formatted.clone(),
            chars: formatted.chars().collect(),
            color_class,
            prefix: String::new(),
            suffix: String::new(),
        }
    }

    pub fn for_pnl(raw_value: Decimal) -> Self {
        if raw_value.is_zero() {
            Self {
                value: "-".to_string(),
                raw: raw_value.to_string(),
                chars: vec!['-'],
                color_class: "text-zinc-500",
                prefix: String::new(),
                suffix: String::new(),
            }
        } else {
            let abs = raw_value.abs();
            let formatted = format_decimal_with_commas(abs, 4);
            let (value, color_class, prefix, suffix) = if raw_value.is_sign_negative() {
                (
                    format!("({formatted})"),
                    "text-red-400",
                    "(".to_string(),
                    ")".to_string(),
                )
            } else {
                (
                    formatted.clone(),
                    "text-emerald-400",
                    String::new(),
                    String::new(),
                )
            };
            Self {
                raw: raw_value.to_string(),
                value,
                chars: formatted.chars().collect(),
                color_class,
                prefix,
                suffix,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimeoutEditorView {
    pub display_text: String,
    pub edit_text: String,
    pub action: String,
    pub error: Option<String>,
}

pub(super) fn format_duration(seconds: i32) -> String {
    let seconds = seconds.max(0) as i64;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3600 {
        let minutes = seconds / 60;
        let rem_seconds = seconds % 60;
        if rem_seconds == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {rem_seconds}s")
        }
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    }
}