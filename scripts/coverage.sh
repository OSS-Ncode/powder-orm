#!/usr/bin/env bash
# Measure test coverage for every Powder component, one language at a time.
#
# Prereqs (installed once):
#   cargo install cargo-llvm-cov && rustup component add llvm-tools
#   <venv>/pip install coverage pytest maturin   (+ maturin develop --release)
#   npx c8 (fetched on demand), go toolchain, JDK + JaCoCo jars (see below)
#
# Usage: scripts/coverage.sh [venv-python]  (defaults to bench-site/.venv)

set -euo pipefail
cd "$(dirname "$0")/.."
PY="${1:-$PWD/bench-site/.venv/Scripts/python.exe}"
[ -f "$PY" ] || PY="${1:-$PWD/bench-site/.venv/bin/python}"

TARGET="$(cargo metadata --format-version 1 --no-deps \
  | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)"

echo "== Rust (powder-core + powder-cli) =="
cargo llvm-cov -p powder-core -p powder-cli --summary-only | tail -3

echo "== Node / TypeScript =="
(cd crates/powder-node \
  && npx --yes c8 --include "dist/**" --exclude "dist/*.test.js" \
     node --test dist/orm.test.js dist/e2e.test.js dist/cache.test.js 2>&1 | tail -8)

echo "== Python =="
(cd crates/powder-python \
  && "$PY" -m coverage run --source=python/powder -m pytest tests -q \
  && "$PY" -m coverage report | tail -3)

echo "== Go =="
(cd bindings/go \
  && POWDER_LIB="$TARGET/release/powder_ffi.dll" go test -cover ./... | tail -2)

echo "== Java (JaCoCo; agent/cli jars in \$JACOCO_DIR, default /tmp/jacoco) =="
JACOCO="${JACOCO_DIR:-/tmp/jacoco}"
if [ -f "$JACOCO/agent.jar" ]; then
  (cd crates/powder-java \
    && java "-javaagent:$JACOCO/agent.jar=destfile=$JACOCO/powder.exec" \
        -cp out PowderTest "$TARGET/release/powder_java.dll" \
    && java -jar "$JACOCO/cli.jar" report "$JACOCO/powder.exec" \
        --classfiles out/com --csv "$JACOCO/report.csv" >/dev/null \
    && "$PY" - "$JACOCO/report.csv" <<'PYEOF'
import csv, sys
rows = list(csv.DictReader(open(sys.argv[1])))
lm = sum(int(r["LINE_MISSED"]) for r in rows)
lc = sum(int(r["LINE_COVERED"]) for r in rows)
print(f"java line coverage: {lc/(lc+lm)*100:.1f}% ({lc}/{lc+lm})")
PYEOF
  )
else
  echo "  (skip: $JACOCO/agent.jar not found — download org.jacoco.agent runtime + org.jacoco.cli nodeps)"
fi
