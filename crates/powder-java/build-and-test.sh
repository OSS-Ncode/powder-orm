#!/usr/bin/env bash
# Build the native cdylib + Java classes and run the JNI e2e test.
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo build -p powder-java --release

# Locate the built native library across platforms / target dirs.
TARGET="$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)"
for name in powder_java.dll libpowder_java.so libpowder_java.dylib; do
  if [ -f "$TARGET/release/$name" ]; then LIB="$TARGET/release/$name"; break; fi
done
if [ -z "${LIB:-}" ]; then echo "native library not found in $TARGET/release" >&2; exit 1; fi

cd crates/powder-java
rm -rf out
javac -d out java/com/powder/*.java java/PowderTest.java
java -cp out PowderTest "$LIB"
