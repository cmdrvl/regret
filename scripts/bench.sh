#!/usr/bin/env bash
set -euo pipefail

ARTIFACT_DIR="${1:-artifacts/bench}"
mkdir -p "$ARTIFACT_DIR"

cargo bench --all-features

if [ -d target/criterion ]; then
  mkdir -p "$ARTIFACT_DIR/criterion"
  cp -R target/criterion/* "$ARTIFACT_DIR/criterion/" 2>/dev/null || true
fi

ARTIFACT_DIR="$ARTIFACT_DIR" python3 - <<'PY'
import glob
import json
import os
from pathlib import Path

artifact_dir = Path(os.environ.get("ARTIFACT_DIR", "artifacts/bench"))
artifact_dir.mkdir(parents=True, exist_ok=True)

files = glob.glob("target/criterion/**/estimates.json", recursive=True)
if not files:
    with open(artifact_dir / "summary.json", "w", encoding="utf-8") as handle:
        json.dump({"status": "no_benchmarks", "count": 0}, handle)
    raise SystemExit(0)

values = []
for path in files:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
    mean = data.get("mean", {}).get("point_estimate")
    if isinstance(mean, (int, float)):
        values.append(float(mean))

if not values:
    with open(artifact_dir / "summary.json", "w", encoding="utf-8") as handle:
        json.dump({"status": "no_estimates", "count": 0}, handle)
    raise SystemExit(0)

mean_value = sum(values) / len(values)
summary = {"status": "ok", "count": len(values), "mean": mean_value}
with open(artifact_dir / "summary.json", "w", encoding="utf-8") as handle:
    json.dump(summary, handle)
PY
