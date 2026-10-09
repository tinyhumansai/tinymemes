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
fn no_marker_appends_best_candidate() {
    let (out, used) = resolve_markers("all good fam", &[meme(1)], 1);
    assert_eq!(used.len(), 1);
    assert!(out.ends_with("![Meme 1](https://m.example/1.gif)"));
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
