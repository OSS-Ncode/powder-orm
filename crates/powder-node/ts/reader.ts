/**
 * Zero-copy reader for the PCB ("Powder Columnar Buffer") wire format.
 *
 * The native addon returns query results as an PCB `Buffer`. This reader
 * parses the fixed header + directory and exposes each column. Numeric columns
 * are surfaced as typed-array *views* directly over the transferred bytes
 * (`Float64Array` / `BigInt64Array`) — no per-value copy — whenever the buffer
 * lands on an 8-byte boundary, which the encoder guarantees for external
 * buffers.
 */

const MAGIC = 0x31424350; // "PCB1" (0x50 0x43 0x42 0x31) read as little-endian u32
const HEADER_LEN = 24;
const COLDIR_LEN = 40;

export enum DataType {
  Int64 = 0,
  Float64 = 1,
  Bool = 2,
  Utf8 = 3,
}

export type Scalar = bigint | number | boolean | string | null;

export interface PowderColumn {
  readonly name: string;
  readonly type: DataType;
  readonly length: number;
  isValid(row: number): boolean;
  get(row: number): Scalar;
  /** Materialize the whole column to a JS array (copies). */
  toArray(): Scalar[];
}

const decoder = new TextDecoder();

/** Read a validity bit (LSB-first). `undefined` bitmap => all valid. */
function validAt(bitmap: Uint8Array | undefined, row: number): boolean {
  if (!bitmap) return true;
  return (bitmap[row >> 3] & (1 << (row & 7))) !== 0;
}

const MIN_SAFE_BIG = BigInt(Number.MIN_SAFE_INTEGER);
const MAX_SAFE_BIG = BigInt(Number.MAX_SAFE_INTEGER);

/**
 * Decode one UTF-8 string out of `data`. `TextDecoder.decode` carries ~100ns
 * of per-call overhead, which dominates columnar string reads; short ASCII
 * runs (the overwhelmingly common case) are built directly instead.
 */
function decodeUtf8(data: Uint8Array, start: number, end: number): string {
  const len = end - start;
  if (len === 0) return "";
  if (len <= 32) {
    let ascii = true;
    let out = "";
    for (let i = start; i < end; i++) {
      const b = data[i];
      if (b >= 0x80) {
        ascii = false;
        break;
      }
      out += String.fromCharCode(b);
    }
    if (ascii) return out;
  }
  return decoder.decode(data.subarray(start, end));
}

class ColumnImpl implements PowderColumn {
  /** Per-instance closure: bounds + validity + read fused into one body, so
   * V8 inlines the raw reader instead of dispatching through a field. */
  readonly get: (row: number) => Scalar;
  /** Like {@link get} without the bounds check — for internal loops whose
   * index is already proven in-range (e.g. toRows). */
  readonly getInRange: (row: number) => Scalar;

  constructor(
    readonly name: string,
    readonly type: DataType,
    readonly length: number,
    private readonly validity: Uint8Array | undefined,
    private readonly readRaw: (row: number) => Scalar,
  ) {
    const length_ = length;
    if (validity === undefined) {
      this.get = (row) => (row >= 0 && row < length_ ? readRaw(row) : null);
      this.getInRange = readRaw;
    } else {
      const bm = validity;
      this.get = (row) =>
        row >= 0 && row < length_ && (bm[row >> 3] & (1 << (row & 7))) !== 0
          ? readRaw(row)
          : null;
      this.getInRange = (row) =>
        (bm[row >> 3] & (1 << (row & 7))) !== 0 ? readRaw(row) : null;
    }
  }

  isValid(row: number): boolean {
    return validAt(this.validity, row);
  }

  toArray(): Scalar[] {
    const out = new Array<Scalar>(this.length);
    for (let i = 0; i < this.length; i++) out[i] = this.get(i);
    return out;
  }
}

export interface PowderBatch {
  readonly numRows: number;
  readonly columns: PowderColumn[];
  column(name: string): PowderColumn | undefined;
  /** Row-oriented view (copies) — convenient for small result sets. */
  toRows(): Record<string, Scalar>[];
}

/** Decode an PCB buffer into a columnar batch. */
export function decodeBatch(buf: Uint8Array): PowderBatch {
  const ab = buf.buffer;
  const base = buf.byteOffset;
  const view = new DataView(ab, base, buf.byteLength);

  if (view.getUint32(0, true) !== MAGIC) {
    throw new Error("not an PCB buffer (bad magic)");
  }
  const version = view.getUint16(4, true);
  if (version !== 1) throw new Error(`unsupported PCB version ${version}`);

  const numColumns = view.getUint32(8, true);
  const numRows = view.getUint32(12, true);
  const dirOff = view.getUint32(16, true);

  // Can we make an aligned typed-array view at absolute offset `off`?
  const aligned = (off: number) => ((base + off) & 7) === 0;

  const columns: PowderColumn[] = [];
  for (let c = 0; c < numColumns; c++) {
    const d = dirOff + c * COLDIR_LEN;
    const nameOff = view.getUint32(d, true);
    const nameLen = view.getUint32(d + 4, true);
    const dtype = view.getUint8(d + 8) as DataType;
    const hasValidity = (view.getUint8(d + 9) & 1) !== 0;
    const validityOff = view.getUint32(d + 12, true);
    const validityLen = view.getUint32(d + 16, true);
    const buf1Off = view.getUint32(d + 20, true);
    const buf2Off = view.getUint32(d + 28, true);
    const buf2Len = view.getUint32(d + 32, true);

    const name = decoder.decode(new Uint8Array(ab, base + nameOff, nameLen));
    const validity = hasValidity
      ? new Uint8Array(ab, base + validityOff, validityLen)
      : undefined;

    let readRaw: (row: number) => Scalar;
    switch (dtype) {
      case DataType.Int64: {
        // Values in the float53-safe range come back as plain `number`,
        // assembled from two u32 halves — no BigInt is ever allocated on the
        // hot path. Only values with |v| ~ 2^53 or wider fall back to bigint.
        if (aligned(buf1Off)) {
          const halves = new Uint32Array(ab, base + buf1Off, numRows * 2);
          let wide: BigInt64Array | undefined;
          readRaw = (r) => {
            const lo = halves[2 * r];
            const hi = halves[2 * r + 1] | 0; // sign-extend the top half
            if (hi > -2097152 && hi < 2097151) return hi * 4294967296 + lo;
            wide ??= new BigInt64Array(ab, base + buf1Off, numRows);
            const v = wide[r];
            return v >= MIN_SAFE_BIG && v <= MAX_SAFE_BIG ? Number(v) : v;
          };
        } else {
          const dv = view;
          readRaw = (r) => {
            const off = buf1Off + r * 8;
            const lo = dv.getUint32(off, true);
            const hi = dv.getInt32(off + 4, true);
            if (hi > -2097152 && hi < 2097151) return hi * 4294967296 + lo;
            const v = dv.getBigInt64(off, true);
            return v >= MIN_SAFE_BIG && v <= MAX_SAFE_BIG ? Number(v) : v;
          };
        }
        break;
      }
      case DataType.Float64: {
        if (aligned(buf1Off)) {
          const arr = new Float64Array(ab, base + buf1Off, numRows);
          readRaw = (r) => arr[r];
        } else {
          const dv = view;
          readRaw = (r) => dv.getFloat64(buf1Off + r * 8, true);
        }
        break;
      }
      case DataType.Bool: {
        const bits = new Uint8Array(ab, base + buf1Off, Math.ceil(numRows / 8));
        readRaw = (r) => (bits[r >> 3] & (1 << (r & 7))) !== 0;
        break;
      }
      case DataType.Utf8: {
        // The encoder 8-byte-aligns buf1, so the offsets buffer is normally a
        // zero-copy Uint32Array view; copy only on the unaligned fallback.
        let offsets: Uint32Array;
        if (((base + buf1Off) & 3) === 0) {
          offsets = new Uint32Array(ab, base + buf1Off, numRows + 1);
        } else {
          offsets = new Uint32Array(numRows + 1);
          for (let i = 0; i <= numRows; i++) {
            offsets[i] = view.getUint32(buf1Off + i * 4, true);
          }
        }
        // Node's Buffer#utf8Slice is a one-shot C++ decode (simdutf) — far
        // cheaper per short string than TextDecoder. Fall back to the portable
        // path outside Node.
        const nodeBuf =
          typeof Buffer !== "undefined" && buf2Len > 0
            ? Buffer.from(ab, base + buf2Off, buf2Len)
            : null;
        // Encoder-stamped pure-ASCII hint (directory flags bit1): decode the
        // whole char-data buffer ONCE, then every row read is an O(1)
        // substring over it (V8 sliced string — no copy, no per-row decode).
        if ((view.getUint8(d + 9) & 2) !== 0 && nodeBuf) {
          // Lazy: the one-shot decode happens on first read, keeping
          // decode-only paths (e.g. cache hits) allocation-free.
          let whole: string | null = null;
          readRaw = (r) =>
            (whole ??= nodeBuf.toString("latin1")).substring(offsets[r], offsets[r + 1]);
          break;
        }
        const utf8Slice =
          nodeBuf && typeof (nodeBuf as unknown as { utf8Slice?: unknown }).utf8Slice === "function"
            ? (nodeBuf as unknown as { utf8Slice(s: number, e: number): string }).utf8Slice.bind(nodeBuf)
            : null;
        // Decoded strings are memoized: the PCB buffer is immutable, so a
        // second pass over the column (e.g. toRows() after a scan) pays no
        // second UTF-8 decode or string allocation. The memo array itself is
        // created on first read so decode-only paths stay allocation-free.
        let memo: (string | undefined)[] | null = null;
        if (utf8Slice) {
          readRaw = (r) => {
            const m = (memo ??= new Array(numRows));
            return m[r] ?? (m[r] = utf8Slice(offsets[r], offsets[r + 1]));
          };
        } else {
          const data = new Uint8Array(ab, base + buf2Off, buf2Len);
          readRaw = (r) => {
            const m = (memo ??= new Array(numRows));
            return m[r] ?? (m[r] = decodeUtf8(data, offsets[r], offsets[r + 1]));
          };
        }
        break;
      }
      default:
        throw new Error(`unsupported PCB type code ${dtype}`);
    }

    columns.push(new ColumnImpl(name, dtype, numRows, validity, readRaw));
  }

  return {
    numRows,
    columns,
    column(name) {
      return columns.find((col) => col.name === name);
    },
    toRows() {
      const rows: Record<string, Scalar>[] = new Array(numRows);
      // A generated factory builds each row as ONE monomorphic object
      // literal (stable hidden class, no per-property dynamic assignment).
      try {
        const args = columns.map((_, i) => `g${i}`);
        const body = `return (r) => ({${columns
          .map((c, i) => `${JSON.stringify(c.name)}: g${i}(r)`)
          .join(", ")}});`;
        const make = new Function(...args, body) as (
          ...fs: unknown[]
        ) => (r: number) => Record<string, Scalar>;
        const rowOf = make(...columns.map((c) => (c as ColumnImpl).getInRange ?? c.get));
        for (let r = 0; r < numRows; r++) {
          rows[r] = rowOf(r);
        }
      } catch {
        // e.g. a CSP forbidding new Function — take the dynamic path.
        for (let r = 0; r < numRows; r++) {
          const row: Record<string, Scalar> = {};
          for (const col of columns) row[col.name] = col.get(r);
          rows[r] = row;
        }
      }
      return rows;
    },
  };
}
