# Extraction and graph checks

Run `cargo test --manifest-path app/core/Cargo.toml knowledge::tests` from repository root.

These automated fixtures verify source-position and exact-quote matching, rejection of invented source IDs/locations, schema rejection, image-region bounds, transactional rejection without partial graph state, idempotence, shared-source evidence preservation, deletion reconciliation, stale edited notes, stable conservative alias merging, Markdown snapshots, and PPTX ordering/notes/tables/images. The direction/condition/negation test checks that distinct fields are preserved; it does **not** establish that a model correctly interpreted the source.

Concept/relation merging requires overlapping normalized names or aliases **and an identical normalized definition** (plus matching relation direction). This deliberately leaves some semantically equivalent rephrasings unmerged. There is no claim that string matching is a general semantic equivalence verifier.

Actual system-framework extraction was run on this Apple Silicon Mac using a release-compiled `Extraction.swift` helper: PNG, JPEG, HEIC, text PDF, scanned PDF, and repeated extraction into an existing cache succeeded; encrypted PDF and corrupt PNG/PDF returned concrete errors. This verifies decoding/OCR/raster production only. It does not verify either subscription provider's interpretation of an image or model output quality. Independent live account evaluation remains required.

## Contracts

- `extract(path)` snapshots the input and returns `input_hash`, `kind`, `units`, and `warnings`.
- `units[].locator`: `kind` (`paragraph`, `page`, `slide`, `image`) and a 1-based `index`.
- `units[].images`: absolute cache image `path` and `mime`.
- `graph.apply`: workspace/material ID, fixed hash, provider/model, and final result matching `../../core/graph_result.schema.json`. The function loads the canonical extraction snapshot, checks the current source, and validates evidence before one SQLite transaction.
- `graph.get`: concepts, relations, assertions with evidence, and source versions. `evidence.immutable_path` opens the analyzed original version. `stale` means the current source differs; no reanalysis is automatic.
- `graph.remove_source(material_id)` removes only that source's support and prunes unsupported assertions/nodes. A graph read also reconciles confirmed storage tombstones after interruption.

Limits: 100 MiB input; 100 PDF pages/PPTX slides; 1,000 extracted units; 512 MiB disposable extraction cache; 128 MiB PNG output per native extraction. Original storage snapshots are separate from this disposable cache. SmartArt and complex slide layout interpretation are explicitly not guaranteed; use PDF for visually significant slide layouts.
