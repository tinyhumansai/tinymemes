use super::*;

#[test]
fn bucket_is_fixed_across_builds() {
    // FNV-1a is specified, so this value must never change; a change would
    // silently move live threads between arms.
    assert_eq!(bucket("thread-abc"), 56);
    assert_eq!(bucket(""), (0xcbf2_9ce4_8422_2325_u64 % 100) as u8);
}

#[test]
fn bucket_stays_in_range() {
    for i in 0..500 {
        assert!(bucket(&format!("thread-{i}")) < 100);
    }
}

#[test]
fn only_real_answers_are_remixed() {
    let placeholder = "You've hit your inference budget.";
    assert!(should_remix("Here's the fix.", placeholder));
    assert!(!should_remix("", placeholder));
    assert!(!should_remix("   \n", placeholder));
    assert!(!should_remix(placeholder, placeholder));
}
