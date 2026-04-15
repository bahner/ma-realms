// Minimal DAG-CBOR decoder for DID documents.
// Handles the CBOR subset used by DAG-CBOR: integers, byte strings,
// text strings, arrays, maps, booleans, null, and CID links (tag 42).

const CBOR_MAJOR_UNSIGNED = 0;
const CBOR_MAJOR_NEGATIVE = 1;
const CBOR_MAJOR_BYTES    = 2;
const CBOR_MAJOR_TEXT     = 3;
const CBOR_MAJOR_ARRAY   = 4;
const CBOR_MAJOR_MAP     = 5;
const CBOR_MAJOR_TAG     = 6;
const CBOR_MAJOR_SIMPLE  = 7;

const textDecoder = new TextDecoder('utf-8');

function readArgument(view, offset, additional) {
  if (additional < 24) return [additional, offset];
  if (additional === 24) return [view.getUint8(offset), offset + 1];
  if (additional === 25) return [view.getUint16(offset, false), offset + 2];
  if (additional === 26) return [view.getUint32(offset, false), offset + 4];
  if (additional === 27) {
    const hi = view.getUint32(offset, false);
    const lo = view.getUint32(offset + 4, false);
    return [hi * 0x100000000 + lo, offset + 8];
  }
  throw new Error(`CBOR: unsupported additional info ${additional}`);
}

function decodeItem(view, data, offset) {
  const initial = view.getUint8(offset);
  offset += 1;
  const major = initial >> 5;
  const additional = initial & 0x1f;

  switch (major) {
    case CBOR_MAJOR_UNSIGNED: {
      const [value, next] = readArgument(view, offset, additional);
      return [value, next];
    }
    case CBOR_MAJOR_NEGATIVE: {
      const [value, next] = readArgument(view, offset, additional);
      return [-1 - value, next];
    }
    case CBOR_MAJOR_BYTES: {
      const [len, next] = readArgument(view, offset, additional);
      return [new Uint8Array(data, next, len), next + len];
    }
    case CBOR_MAJOR_TEXT: {
      const [len, next] = readArgument(view, offset, additional);
      return [textDecoder.decode(new Uint8Array(data, next, len)), next + len];
    }
    case CBOR_MAJOR_ARRAY: {
      const [len, next] = readArgument(view, offset, additional);
      const arr = [];
      let pos = next;
      for (let i = 0; i < len; i++) {
        const [item, nextPos] = decodeItem(view, data, pos);
        arr.push(item);
        pos = nextPos;
      }
      return [arr, pos];
    }
    case CBOR_MAJOR_MAP: {
      const [len, next] = readArgument(view, offset, additional);
      const obj = {};
      let pos = next;
      for (let i = 0; i < len; i++) {
        const [key, kPos] = decodeItem(view, data, pos);
        const [val, vPos] = decodeItem(view, data, kPos);
        obj[key] = val;
        pos = vPos;
      }
      return [obj, pos];
    }
    case CBOR_MAJOR_TAG: {
      const [tag, next] = readArgument(view, offset, additional);
      const [value, vNext] = decodeItem(view, data, next);
      if (tag === 42 && value instanceof Uint8Array) {
        // DAG-CBOR CID link — return as {'/': '<base-encoded-cid>'}
        return [{ '/': bytesToCidString(value) }, vNext];
      }
      return [value, vNext];
    }
    case CBOR_MAJOR_SIMPLE: {
      if (additional === 20) return [false, offset];
      if (additional === 21) return [true, offset];
      if (additional === 22) return [null, offset];
      if (additional === 23) return [undefined, offset];
      if (additional === 25 || additional === 26 || additional === 27) {
        // float16/32/64 — not expected in DID docs, skip with size
        const sizes = { 25: 2, 26: 4, 27: 8 };
        const buf = new DataView(data, offset, sizes[additional]);
        const val = additional === 26
          ? buf.getFloat32(0, false)
          : buf.getFloat64(0, false);
        return [val, offset + sizes[additional]];
      }
      throw new Error(`CBOR: unsupported simple value ${additional}`);
    }
    default:
      throw new Error(`CBOR: unknown major type ${major}`);
  }
}

function bytesToCidString(bytes) {
  // Minimal: return hex-encoded for diagnostics. CID links in DID docs are rare.
  return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Decode a DAG-CBOR payload (ArrayBuffer or Uint8Array) into a JS object.
 */
export function decodeDagCbor(input) {
  const buffer = input instanceof ArrayBuffer ? input : input.buffer.slice(input.byteOffset, input.byteOffset + input.byteLength);
  const view = new DataView(buffer);
  const [result] = decodeItem(view, buffer, 0);
  return result;
}
