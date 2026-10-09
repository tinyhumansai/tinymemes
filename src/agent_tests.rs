use super::*;

fn meme(n: usize) -> Meme {
    Meme {
        title: format!("Meme {n}"),
        url: format!("https://m.example/{n}.gif"),
        source: "test".into(),
        meaning: None,
    }
}

#[test]
fn markers_resolve_with_cap_and_dedupe() {
    let c = [meme(1), meme(2), meme(3)];
    let (out, used) = resolve_markers(
        "yo\n[[meme:2]]\nmid [[meme:2]] [[meme:9]] [[meme:3]] [[meme:1]]\nend",
        &c,
        2,
    );
    assert_eq!(used, vec![meme(2), meme(3)]);
    assert!(out.contains("![Meme 2](https://m.example/2.gif)"));
    assert!(out.contains("![Meme 3](https://m.example/3.gif)"));
    assert!(!out.contains("[[meme"));
    assert!(!out.contains("\n\n\n"));
}

#[test]
fn no_marker_means_no_meme() {
    let (out, used) = resolve_markers("all good fam", &[meme(1)], 1);
    assert!(used.is_empty());
    assert_eq!(out, "all good fam");
}

#[test]
fn zero_budget_strips_markers() {
    let (out, used) = resolve_markers("hey [[meme:1]] there", &[meme(1)], 0);
    assert!(used.is_empty());
    assert_eq!(out, "hey  there");
}

#[test]
fn protected_content_must_survive() {
    let original = "Run `cargo test` then see https://docs.rs/x.\n```rust\nfn main() {}\n```";
    assert!(preserves_protected(
        original,
        "ngl just `cargo test` fr, peep https://docs.rs/x\n```rust\nfn main() {}\n```"
    ));
    assert!(!preserves_protected(
        original,
        "just run the tests bestie https://docs.rs/x\n```rust\nfn main() {}\n```"
    ));
    assert!(!preserves_protected(
        original,
        "`cargo test` https://docs.rs/x\n```rust\nfn main(){}\n```"
    ));
}

#[test]
fn ballooned_rewrite_is_rejected() {
    let original = "x".repeat(300);
    assert!(within_length(&original, &"y".repeat(700)));
    assert!(!within_length(&original, &"y".repeat(900)));
}

#[test]
fn wrapping_fence_is_stripped() {
    assert_eq!(strip_wrapping_fence("```markdown\nyo\n```"), "yo");
    assert_eq!(
        strip_wrapping_fence("```rust\nfn x(){}\n```"),
        "```rust\nfn x(){}\n```"
    );
}

#[test]
fn duplicate_from_the_live_run_is_repaired_keeping_the_original_line() {
    let original = "🤣🤣 bhai yeh toh classic hai!\n\n**Roast time:**\n\n- \"Bro\" bolke tune apne promotion ke chances bhi bro-friendzone kar diye 💀\n\n- Ab tera annual review: \"Outstanding performance par bro nahi bolna tha\"";
    let rewritten = "🤣🤣 arre bhai yeh toh historical moment hai!\n\nBoss ko \"bro\" bolke tune apne promotion ke chances bhi bro-friendzone kar diye 💀\n\n**Roast ka time:**\n- \"Bro\" bolke tune apne promotion ke chances bhi bro-friendzone kar diye 💀\n- Ab tera annual review: \"Outstanding performance par bro nahi bolna tha\"";
    let (out, removed) = repair_duplicates(original, rewritten);
    assert_eq!(removed, 1);
    assert_eq!(out.matches("bro-friendzone").count(), 1);
    // The copy kept is the list item that mirrors the original line.
    assert!(out.contains("- \"Bro\" bolke tune"));
    assert!(out.contains("historical moment"));
    assert!(out.contains("annual review"));
}

#[test]
fn repeats_the_original_already_had_are_left_alone() {
    let original = "Run the tests again after every change you make\nRun the tests again after every change you make";
    let (out, removed) = repair_duplicates(original, original);
    assert_eq!(removed, 0);
    assert_eq!(out, original);
}

#[test]
fn short_lines_are_never_treated_as_duplicates() {
    let (_, removed) = repair_duplicates("", "lol\nlol\nscene set hai\nscene set hai");
    assert_eq!(removed, 0);
}

#[test]
fn meme_only_puts_the_meme_after_the_first_paragraph() {
    let (out, used) = attach_meme("Big win bro!\n\nNow sign the offer letter.", &[meme(1)], 1);
    assert_eq!(used.len(), 1);
    let pos_meme = out.find("![Meme 1]").unwrap();
    assert!(pos_meme > out.find("Big win").unwrap() && pos_meme < out.find("Now sign").unwrap());
    let (out, used) = attach_meme("One paragraph only.", &[meme(1)], 0);
    assert!(used.is_empty());
    assert_eq!(out, "One paragraph only.");
}
