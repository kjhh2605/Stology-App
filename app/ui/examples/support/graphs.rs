//! Fixed graphs shared by the map examples. No user data.
use serde_json::{json, Value};

/// The small biology graph used by the installed-app checks.
pub fn biology() -> Value {
    json!({
        "concepts": [
            {"id": "photosynthesis", "name": "광합성", "aliases": ["Photosynthesis"]},
            {"id": "chloroplast", "name": "엽록체"},
            {"id": "atp", "name": "ATP"},
            {"id": "mitochondria", "name": "미토콘드리아"},
            {"id": "organelle", "name": "세포 소기관"},
            {"id": "square", "name": "주황색 정사각형"},
            {"id": "circle", "name": "보라색 원"}
        ],
        "relations": [
            {"id": "occurs", "label": "에서 일어난다", "directed": true},
            {"id": "makes", "label": "생성한다", "directed": true},
            {"id": "is", "label": "이다", "directed": true},
            {"id": "site", "label": "발생 장소이다", "directed": true},
            {"id": "role", "label": "역할로 제시된다", "directed": true},
            {"id": "points", "label": "화살표로 가리킨다", "directed": true}
        ],
        "assertions": [
            {"id": "a1", "subject": "photosynthesis", "object": "chloroplast", "relation": "occurs", "condition": "빛이 있을 때", "evidence": []},
            {"id": "a2", "subject": "chloroplast", "object": "photosynthesis", "relation": "site", "evidence": []},
            {"id": "a3", "subject": "chloroplast", "object": "photosynthesis", "relation": "role", "evidence": []},
            {"id": "a4", "subject": "mitochondria", "object": "atp", "relation": "makes", "evidence": []},
            {"id": "a5", "subject": "chloroplast", "object": "organelle", "relation": "is", "evidence": []},
            {"id": "a6", "subject": "square", "object": "circle", "relation": "points", "evidence": []}
        ]
    })
}
/// A dense graph: 300 concepts in loose communities.
pub fn dense() -> Value {
    let count = 300usize;
    let concepts = (0..count)
        .map(|i| json!({"id": format!("n{i:03}"), "name": format!("개념 {i}")}))
        .collect::<Vec<_>>();
    let mut assertions = Vec::new();
    let mut state = 2026u64;
    let mut random = |bound: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % bound
    };
    for i in 1..count {
        // Mostly within a community of 25, sometimes across.
        let home = i / 25 * 25;
        let other = if random(5) == 0 {
            random(i)
        } else {
            home + random((i - home).max(1))
        };
        assertions.push(json!({"id": format!("e{i}"), "subject": format!("n{i:03}"), "object": format!("n{other:03}"), "relation": if i % 3 == 0 { "uses" } else { "part" }, "evidence": []}));
    }
    for i in 0..14 {
        assertions.push(json!({"id": format!("hub{i}"), "subject": "n000", "object": format!("n{:03}", 30 + i * 17), "relation": "uses", "evidence": []}));
    }
    json!({
        "concepts": concepts,
        "relations": [
            {"id": "part", "label": "의 일부이다", "directed": true},
            {"id": "uses", "label": "사용한다", "directed": false}
        ],
        "assertions": assertions
    })
}
