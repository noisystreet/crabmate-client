//! `--connect name=url` 的解析边界。
//!
//! 独立成文件：`main.rs` 的行数贴着 `fn-nloc-ratchet` 的 920 行/文件上限，
//! 而这几条纯解析用例与 CLI 其余部分没有耦合。

use crate::parse_connect_spec;

#[test]
fn parses_name_and_url() {
    let (name, url) = parse_connect_spec("prod=http://10.0.0.5:8080").unwrap();
    assert_eq!(name, "prod");
    assert_eq!(url, "http://10.0.0.5:8080");
}

#[test]
fn empty_name_falls_back_to_the_url() {
    // 手敲长 URL 时没人愿意再编个名字，直接用 URL 至少能认出来。
    let (name, url) = parse_connect_spec("=http://x:1").unwrap();
    assert_eq!(name, "http://x:1");
    assert_eq!(url, "http://x:1");
}

#[test]
fn missing_separator_is_an_error() {
    // 光给 URL 是常见手误：不静默当成「只有 url」，否则 lane 名字会是一片空白。
    assert!(parse_connect_spec("http://x:1").is_err());
}

#[test]
fn empty_url_is_an_error() {
    assert!(parse_connect_spec("prod=").is_err());
    assert!(parse_connect_spec("prod=   ").is_err());
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    let (name, url) = parse_connect_spec("  prod  =  http://x:1  ").unwrap();
    assert_eq!(name, "prod");
    assert_eq!(url, "http://x:1");
}
