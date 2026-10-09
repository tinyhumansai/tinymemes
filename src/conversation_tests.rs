use super::*;

#[test]
fn window_keeps_latest_turns_and_clips() {
    let convo: Vec<Turn> = (0..30)
        .map(|i| Turn::user(format!("msg {i} {}", "x".repeat(50))))
        .collect();
    let state = jev_state(
        &convo,
        "reply",
        "India",
        Window {
            max_turns: 3,
            max_chars_per_turn: 10,
        },
    );
    let turns = state["conversation"].as_array().unwrap();
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0]["text"], "msg 27 xxx…");
    assert_eq!(state["assistant_reply"], "reply");
}

#[test]
fn memes_become_text_and_remixed_turns_are_flagged() {
    let (text, memes) = memes_as_text(
        "lol\n\n![Moye Moye](https://i.imgflip.com/82yaur.png)\n\nok [meme: Mast Plan Hai]",
    );
    assert_eq!(text, "lol\n\n[meme: Moye Moye]\n\nok [meme: Mast Plan Hai]");
    assert_eq!(memes.len(), 2);
    assert_eq!(
        memes[0].url.as_deref(),
        Some("https://i.imgflip.com/82yaur.png")
    );
    let state = jev_state(
        &[
            Turn::user("hi"),
            Turn::remixed("arre ![X](https://a/x.png)"),
        ],
        "r",
        "India",
        Window::default(),
    );
    let turns = state["conversation"].as_array().unwrap();
    assert!(turns[0].get("note").is_none());
    assert_eq!(turns[1]["text"], "arre [meme: X]");
    assert!(turns[1]["note"].as_str().unwrap().contains("not evidence"));
}
