//! Temporary probe: does CI fail when a test fails?
#[test]
fn deliberate_failure() {
    assert_eq!(1, 2, "CI must not be green when a test fails");
}
