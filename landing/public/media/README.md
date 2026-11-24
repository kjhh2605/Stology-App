# 랜딩사이트 이미지 자료

기능 소개 장면은 영상 파일을 쓰지 않는다. 노트 16개·PDF 13개 장면은 실제 앱 UI를 재현한 DOM/CSS 장면을, 06 과목 지도는 앱의 지식지도를 옮긴 Canvas 2D 장면을 브라우저에서 바로 그린다(`landing/src/scenes/`). 이 폴더에는 실제 앱 화면 이미지와 히어로 자산만 남는다. 06 섹션에 쓰던 캡처 `map-overview.jpg`는 2026-11-24 장면으로 바꾸면서 지웠다([ADR-040](../../../decisions/2026-11-24/ADR-040-landing-course-map-canvas-scene.md)).

| 파일 | 용도 | 출처 |
|---|---|---|
| `app-overview.jpg` | 앱 창 전체의 기준 이미지. DOM 재현의 대조용 | 2026-11-17 시험용 Stology 캡처 |
| `pdf-overview.jpg` | PDF 필기 화면의 기준 이미지. 도구 막대 순서·필기 색의 출처 | 2026-11-17 시험용 Stology 캡처 |
| `map-graph-crop.jpg` | 노트 장면 '지도 검색창으로 이동'의 배경(지도 영역만 잘라 냄, 515×505) | [2026-11-18 과목 정리 검증 캡처](../../../app/qa/runs/subject-ontology-2026-11-18/map-claude-workspace.jpeg)에서 `sips`로 잘라 냄 |
| `hero/star.webp`, `hero/star-sheet.webp` | 라벤더 별 캐릭터 스프라이트 | Blender 원본 렌더([스크립트](../../scripts/render-star-sprites.py)) |
| `intro/stology-intro-ko.mp4` | 마무리 섹션의 35초 소개 영상(1080p·60fps·H.264+AAC, moov 앞쪽, 7.3 MB). 다시 인코딩하지 않고 그대로 복사 | 영상 워크트리의 `Stology_소개영상_37초_Claude.mp4`([ADR-041](../../../decisions/2026-11-24/ADR-041-landing-closing-intro-video.md)) |
| `intro/poster.jpg` | 소개 영상 포스터, 25초 지점 1280×720 | 같은 영상에서 `ffmpeg -ss 25`로 뽑음 |
| `psychology-demo.pdf` | 시연 자료 원본(심리학의 이해) | 직접 작성 |

## 이전 녹화 자산 (이력)

2026-11-17~22의 랜딩은 `demos/<id>.mp4`·`.jpg` 29쌍의 실제 앱 녹화를 자동 반복 재생했다([ADR-025](../../../decisions/2026-11-17/ADR-025-passive-native-demos.md)). 2026-11-23 재구축에서 사용자의 요청에 따라 전부 DOM 장면으로 바꾸면서 녹화 파일, `manifest.json`, 인코더 `encode-recordings.py`를 저장소에서 제거했다. 필요하면 Git 이력(2026-11-23 이전 커밋)에서 꺼낼 수 있다. 녹화 방식과 편집 구간 기록은 그 시점의 이 문서에 있다.
