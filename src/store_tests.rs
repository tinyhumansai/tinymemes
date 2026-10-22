use super::*;

#[test]
fn remixed_history_keeps_only_the_newest_ids() {
    let mut ids: VecDeque<String> = (0..REMIXED_CAP + 50).map(|i| format!("id-{i}")).collect();
    cap_remixed(&mut ids);
    assert_eq!(ids.len(), REMIXED_CAP);
    assert_eq!(ids.front().map(String::as_str), Some("id-50"));
    assert_eq!(
        ids.back().map(String::as_str),
        Some(format!("id-{}", REMIXED_CAP + 49).as_str())
    );
}
