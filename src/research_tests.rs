use super::*;
use std::sync::Mutex;

struct Hits(Mutex<Vec<String>>);

#[async_trait]
impl WebSearch for Hits {
    async fn search(&self, query: &str, _max: usize) -> Result<Vec<SearchHit>, BoxError> {
        self.0.lock().unwrap().push(query.to_owned());
        Ok(vec![SearchHit {
            title: "Hinglish slang guide".into(),
            url: "https://slang.example/guide/".into(),
            snippet: Some("bawaal: chaos; scene set hai: all arranged".into()),
        }])
    }
}

struct Model;

#[async_trait]
impl ChatModel for Model {
    async fn complete(&self, system: &str, _user: &str) -> Result<String, BoxError> {
        if system.starts_with("Turn the request") {
            return Ok("hinglish slang for job offer".into());
        }
        Ok(r#"{"terms": [
            {"term": "bawaal", "meaning": "chaos", "intensity": "spicy", "source_url": "https://slang.example/guide"},
            {"term": "made up", "meaning": "x", "intensity": "light", "source_url": "https://elsewhere.example/"}
        ]}"#
        .into())
    }
}

#[tokio::test]
async fn long_requests_are_condensed_and_terms_are_grounded_in_results() {
    let search = Arc::new(Hits(Mutex::new(Vec::new())));
    let r = SearchResearcher::new(search.clone(), Arc::new(Model));
    let request = format!(
        "Hinglish slang that fits this message: \"{}\"",
        "x".repeat(200)
    );
    let found = r.research(&request).await.unwrap();
    assert_eq!(
        search.0.lock().unwrap().as_slice(),
        ["hinglish slang for job offer"]
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].term, "bawaal");
    assert_eq!(found[0].min_tier, Tier::Spicy);
}
