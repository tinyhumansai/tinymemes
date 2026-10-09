use super::*;

/// Trimmed from a real GIPHY page (2026-10-09): meta tags plus embedded data
/// holding this GIF's record and a related GIF's tags.
const PAGE: &str = r#"<html><head>
<meta property="og:title" content="Tmkoc GIF - Find &amp; Share on GIPHY"/>
<meta property="og:image" content="https://media1.giphy.com/media/v1.Y2lkPTc5MGI3NjEx/SinIpWR8Jc6VVIdwNy/giphy.webp"/>
</head><body><script>self.__next_f.push([1,"{\"gifs\":[{\"id\":\"OTHER1\",\"rating\":\"pg\",\"tags\":[\"jethalal hairs\",\"tmkoc hairs\"]},{\"id\":\"SinIpWR8Jc6VVIdwNy\",\"rating\":\"g\",\"tags\":[\"giphycreatortest\",\"tmkoc\",\"jethalal\",\"chup ho ja satvi fail\"]}]}"])</script></body></html>"#;

const URL: &str = "https://giphy.com/gifs/tmkoc-jethalal-chup-ho-ja-satvi-fail-SinIpWR8Jc6VVIdwNy";

#[test]
fn gif_ids_come_only_from_single_gif_pages() {
    assert_eq!(giphy_gif_id(URL), Some("SinIpWR8Jc6VVIdwNy"));
    assert_eq!(giphy_gif_id("https://giphy.com/explore/indian"), None);
    assert_eq!(
        giphy_gif_id("https://tenor.com/search/indian-party-gifs"),
        None
    );
}

#[test]
fn a_real_page_yields_small_media_rating_tags_and_slug() {
    let found = parse_giphy_page(URL, PAGE).unwrap();
    assert_eq!(found.title, "Tmkoc GIF");
    assert_eq!(
        found.media_url,
        "https://media1.giphy.com/media/v1.Y2lkPTc5MGI3NjEx/SinIpWR8Jc6VVIdwNy/200.webp"
    );
    // This GIF's own record, not the related GIF listed first.
    assert_eq!(found.rating.as_deref(), Some("g"));
    assert!(found.tags.contains(&"chup ho ja satvi fail".to_owned()));
    assert!(!found.tags.contains(&"tmkoc hairs".to_owned()));
    assert_eq!(
        found.slug_words,
        ["tmkoc", "jethalal", "chup", "ho", "ja", "satvi", "fail"]
    );
}

#[test]
fn media_from_other_hosts_is_refused() {
    let page = PAGE.replace("media1.giphy.com", "evil.example");
    assert!(parse_giphy_page(URL, &page).is_none());
}
