#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cargo test --locked --manifest-path "$ROOT/core/Cargo.toml"
cargo test --locked --manifest-path "$ROOT/ui/Cargo.toml"
# decisions/ is a local-only folder; check it when present.
if [ -f "$ROOT/../decisions/tools/check_decisions.py" ]; then
  python3 "$ROOT/../decisions/tools/check_decisions.py"
fi
