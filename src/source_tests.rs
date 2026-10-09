use super::*;

fn template(name: &str) -> Meme {
    Meme {
        title: name.to_owned(),
        url: format!("https://i.imgflip.com/{}.jpg", name.len()),
        source: "imgflip".into(),
        meaning: None,
    }
}

#[tokio::test]
async fn imgflip_prefers_intent_hints_then_query_words() {
    let src = Imgflip::with_templates(vec![
        template("Drake Hotline Bling"),
        template("This Is Fine"),
        template("Fine Dining Cat"),
        template("Two Buttons"),
    ]);
    let hits = src
        .search(
            "everything is fine",
            Intent::Frustration,
            &Region::global(),
            5,
        )
        .await
        .unwrap();
    let names: Vec<_> = hits.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(names, ["This Is Fine", "Fine Dining Cat"]);
}
