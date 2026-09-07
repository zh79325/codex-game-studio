use super::*;
use pretty_assertions::assert_eq;

/// The audit trail must identify which credential was used without ever
/// recording a usable one.
#[test]
fn masked_bearer_identifies_the_key_without_exposing_it() {
    let api_key = "tsk_live_0123456789abcdefghijklmnop";
    let masked = mask_bearer(api_key);

    assert!(!masked.contains(api_key));
    assert!(!masked.contains("0123456789"));
    assert!(masked.starts_with("Bearer ••••mnop"));
    assert!(masked.contains("chars=35"));

    // The digest pins the key identity, so two different keys never look alike.
    assert_ne!(masked, mask_bearer("tsk_live_something_else_entirely_mnop"));
    assert_eq!(masked, mask_bearer(api_key));
}

#[test]
fn masked_bearer_reports_a_missing_key() {
    assert_eq!(mask_bearer("   "), "Bearer <missing>");
}
