use super::*;

#[test]
fn animated_number_formats_with_thousands_separators() {
    let number =
        AnimatedNumber::from_decimal(rust_decimal::Decimal::new(1234567, 0), "text-zinc-100");
    assert_eq!(number.value, "1,234,567.0000");
    assert_eq!(number.chars.iter().collect::<String>(), "1,234,567.0000");
}

#[test]
fn money_formatters_support_custom_decimal_places() {
    assert_eq!(
        format_money_text_with_decimals(Some(rust_decimal::Decimal::new(1234567, 0)), 0),
        "1,234,567"
    );
    assert_eq!(
        format_neutral_money_cell_with_decimals(Some(rust_decimal::Decimal::new(31000, 0)), 0)
            .value,
        "31,000"
    );
}
