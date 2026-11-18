use serde_json::Value;
use std::collections::HashSet;
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}
pub fn matches(data: &Value, query: &str) -> Vec<Value> {
    let q = normalize(query);
    let mut result: Vec<Value> = data["concepts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            normalize(c["name"].as_str().unwrap_or("")).contains(&q)
                || c["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|a| normalize(a.as_str().unwrap_or("")).contains(&q))
        })
        .cloned()
        .collect();
    result.sort_by_key(|c| {
        let name = normalize(c["name"].as_str().unwrap_or(""));
        let exact_alias = c["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|a| normalize(a.as_str().unwrap_or("")) == q);
        (
            if name == q {
                0
            } else if exact_alias {
                1
            } else {
                2
            },
            name,
        )
    });
    result
}
pub fn degree(data: &Value, id: &str) -> usize {
    neighbors(data, id).len()
}
pub fn neighbors(data: &Value, id: &str) -> Vec<Value> {
    let ids: HashSet<_> = data["assertions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            if a["subject"] == id {
                a["object"].as_str()
            } else if a["object"] == id {
                a["subject"].as_str()
            } else {
                None
            }
        })
        .collect();
    data["concepts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["id"].as_str().is_some_and(|id| ids.contains(id)))
        .cloned()
        .collect()
}
pub fn sorted(data: &Value, selected: &str) -> Vec<Value> {
    let mut links: std::collections::HashMap<&str, HashSet<&str>> =
        std::collections::HashMap::new();
    for a in data["assertions"].as_array().into_iter().flatten() {
        if let (Some(from), Some(to)) = (a["subject"].as_str(), a["object"].as_str()) {
            links.entry(from).or_default().insert(to);
            links.entry(to).or_default().insert(from);
        }
    }
    let mut result = data["concepts"].as_array().cloned().unwrap_or_default();
    result.sort_by(|a, b| {
        let aid = a["id"].as_str().unwrap_or("");
        let bid = b["id"].as_str().unwrap_or("");
        (bid == selected)
            .cmp(&(aid == selected))
            .then_with(|| {
                links
                    .get(bid)
                    .map(HashSet::len)
                    .unwrap_or(0)
                    .cmp(&links.get(aid).map(HashSet::len).unwrap_or(0))
            })
            .then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            })
    });
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn korean_alias_search_ignores_spacing_and_case() {
        let d = json!({"concepts":[{"id":"c","name":"인공 지능","aliases":["Artificial Intelligence"]}]});
        assert_eq!(matches(&d, "인공지능").len(), 1);
        assert_eq!(matches(&d, "ARTIFICIALintel").len(), 1);
    }
    #[test]
    fn selected_search_name_wins_over_substring() {
        let data = serde_json::json!({"concepts":[{"id":"synthase","name":"ATP synthase"},{"id":"atp","name":"ATP"}]});
        assert_eq!(matches(&data, "ATP")[0]["id"], "atp");
    }
    #[test]
    fn neighbor_degree_deduplicates_claims_and_keeps_direction() {
        let d = json!({"concepts":[{"id":"a"},{"id":"b"},{"id":"c"}],"assertions":[{"subject":"a","object":"b"},{"subject":"a","object":"b"},{"subject":"c","object":"a"}]});
        assert_eq!(degree(&d, "a"), 2);
        assert_eq!(degree(&d, "b"), 1);
    }
}
