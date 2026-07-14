use super::{super::model_catalog::body_looks_like_svg, *};

#[test]
fn parses_valid_currency_logo_filename() {
    assert_eq!(parse_currency_logo_filename("BTC.svg").unwrap(), "BTC");
}

#[test]
fn rejects_invalid_currency_logo_filenames() {
    for filename in [
        ".svg",
        "BTC",
        "BTC.png",
        "BTC.svg.svg",
        "../BTC.svg",
        "BTC/ETH.svg",
    ] {
        assert!(
            parse_currency_logo_filename(filename).is_err(),
            "{filename}"
        );
    }
}

#[test]
fn fallback_contains_complete_currency_identifier() {
    let fallback = fallback_currency_logo_svg("XYZ");
    assert!(fallback.contains("<svg"));
    assert!(fallback.contains(">XYZ<"));
}

#[test]
fn svg_validator_requires_safe_namespace_and_content() {
    let valid = br##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#icon"/></svg>"##;
    assert!(body_looks_like_svg(valid));
    for invalid in [
        b"not xml".as_slice(),
        br#"<html xmlns="http://www.w3.org/2000/svg"/>"#.as_slice(),
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#.as_slice(),
        br#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject/></svg>"#.as_slice(),
        br#"<svg xmlns="http://www.w3.org/2000/svg" onload="x"/>"#.as_slice(),
        br#"<svg xmlns="http://www.w3.org/2000/svg"><use href="https://evil.test/x"/></svg>"#
            .as_slice(),
    ] {
        assert!(!body_looks_like_svg(invalid));
    }
}
