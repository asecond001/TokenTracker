import { createElement, type CSSProperties } from 'react';

interface QrVersionSpec {
  version: number;
  totalCodewords: number;
  ecPerBlock: number;
  g1Blocks: number;
  g1Data: number;
  g2Blocks: number;
  g2Data: number;
  alignCenters: readonly number[];
}

// ISO/IEC 18004 Table 9 — ECC Level L (01), Versions 1..12 (up to 367 UTF-8 bytes)
const QR_LEVEL_L_SPECS: readonly QrVersionSpec[] = [
  { version: 1, totalCodewords: 26, ecPerBlock: 7, g1Blocks: 1, g1Data: 19, g2Blocks: 0, g2Data: 0, alignCenters: [] },
  { version: 2, totalCodewords: 44, ecPerBlock: 10, g1Blocks: 1, g1Data: 34, g2Blocks: 0, g2Data: 0, alignCenters: [6, 18] },
  { version: 3, totalCodewords: 70, ecPerBlock: 15, g1Blocks: 1, g1Data: 55, g2Blocks: 0, g2Data: 0, alignCenters: [6, 22] },
  { version: 4, totalCodewords: 100, ecPerBlock: 20, g1Blocks: 1, g1Data: 80, g2Blocks: 0, g2Data: 0, alignCenters: [6, 26] },
  { version: 5, totalCodewords: 134, ecPerBlock: 26, g1Blocks: 1, g1Data: 108, g2Blocks: 0, g2Data: 0, alignCenters: [6, 30] },
  { version: 6, totalCodewords: 172, ecPerBlock: 18, g1Blocks: 2, g1Data: 68, g2Blocks: 0, g2Data: 0, alignCenters: [6, 34] },
  { version: 7, totalCodewords: 196, ecPerBlock: 20, g1Blocks: 2, g1Data: 78, g2Blocks: 0, g2Data: 0, alignCenters: [6, 22, 38] },
  { version: 8, totalCodewords: 242, ecPerBlock: 24, g1Blocks: 2, g1Data: 97, g2Blocks: 0, g2Data: 0, alignCenters: [6, 24, 42] },
  { version: 9, totalCodewords: 292, ecPerBlock: 30, g1Blocks: 2, g1Data: 116, g2Blocks: 0, g2Data: 0, alignCenters: [6, 26, 46] },
  { version: 10, totalCodewords: 346, ecPerBlock: 18, g1Blocks: 2, g1Data: 68, g2Blocks: 2, g2Data: 69, alignCenters: [6, 28, 50] },
  { version: 11, totalCodewords: 404, ecPerBlock: 20, g1Blocks: 4, g1Data: 81, g2Blocks: 0, g2Data: 0, alignCenters: [6, 30, 54] },
  { version: 12, totalCodewords: 466, ecPerBlock: 24, g1Blocks: 2, g1Data: 92, g2Blocks: 2, g2Data: 93, alignCenters: [6, 32, 58] },
];

const GF_EXP = new Uint8Array(512);
const GF_LOG = new Uint8Array(256);

(() => {
  let x = 1;
  for (let i = 0; i < 255; i += 1) {
    GF_EXP[i] = x;
    GF_LOG[x] = i;
    x <<= 1;
    if (x & 0x100) {
      x ^= 0x11d;
    }
  }
  for (let i = 255; i < 512; i += 1) {
    GF_EXP[i] = GF_EXP[i - 255];
  }
})();

export function gfMul(a: number, b: number): number {
  if (a === 0 || b === 0) return 0;
  return GF_EXP[GF_LOG[a] + GF_LOG[b]];
}

function rsGeneratorPoly(degree: number): Uint8Array {
  let poly = new Uint8Array([1]);
  for (let i = 0; i < degree; i += 1) {
    const next = new Uint8Array(poly.length + 1);
    const factor = GF_EXP[i];
    for (let j = 0; j < poly.length; j += 1) {
      next[j] ^= poly[j];
      next[j + 1] ^= gfMul(poly[j], factor);
    }
    poly = next;
  }
  return poly;
}

export function rsComputeRemainder(data: Uint8Array, ecCount: number): Uint8Array {
  const gen = rsGeneratorPoly(ecCount);
  const work = new Uint8Array(data.length + ecCount);
  work.set(data);
  for (let i = 0; i < data.length; i += 1) {
    const lead = work[i];
    if (lead !== 0) {
      for (let j = 0; j < gen.length; j += 1) {
        work[i + j] ^= gfMul(gen[j], lead);
      }
    }
  }
  return work.subarray(data.length);
}

function computeBchFormatInfo(mask: number): number {
  // ECC Level L indicator = 01 (bits 4..3), mask pattern = 0..7 (bits 2..0)
  const data = (0b01 << 3) | (mask & 0x7);
  let rem = data << 10;
  for (let i = 14; i >= 10; i -= 1) {
    if ((rem >> i) & 1) {
      rem ^= 0x537 << (i - 10);
    }
  }
  return ((data << 10) | rem) ^ 0x5412;
}

function computeBchVersionInfo(version: number): number {
  let rem = version << 12;
  for (let i = 17; i >= 12; i -= 1) {
    if ((rem >> i) & 1) {
      rem ^= 0x1f25 << (i - 12);
    }
  }
  return (version << 12) | rem;
}

function selectSpecForPayload(byteLen: number): QrVersionSpec {
  for (const spec of QR_LEVEL_L_SPECS) {
    const totalDataCodewords = spec.g1Blocks * spec.g1Data + spec.g2Blocks * spec.g2Data;
    const countBits = spec.version <= 9 ? 8 : 16;
    const capacityBits = totalDataCodewords * 8;
    if (4 + countBits + byteLen * 8 <= capacityBits) {
      return spec;
    }
  }
  throw new Error(`Payload too large for QR v1..v12 (${byteLen} bytes)`);
}

function encodeDataCodewords(bytes: Uint8Array, spec: QrVersionSpec): Uint8Array {
  const totalDataCodewords = spec.g1Blocks * spec.g1Data + spec.g2Blocks * spec.g2Data;
  const countBits = spec.version <= 9 ? 8 : 16;
  const bits: number[] = [];

  const pushBits = (val: number, len: number) => {
    for (let i = len - 1; i >= 0; i -= 1) {
      bits.push((val >> i) & 1);
    }
  };

  // Mode: Byte = 0100
  pushBits(0b0100, 4);
  pushBits(bytes.length, countBits);
  for (const b of bytes) {
    pushBits(b, 8);
  }

  const totalBits = totalDataCodewords * 8;
  const terminatorLen = Math.min(4, totalBits - bits.length);
  for (let i = 0; i < terminatorLen; i += 1) {
    bits.push(0);
  }
  while (bits.length % 8 !== 0) {
    bits.push(0);
  }

  const out = new Uint8Array(totalDataCodewords);
  for (let i = 0; i < bits.length / 8; i += 1) {
    let val = 0;
    for (let b = 0; b < 8; b += 1) {
      val = (val << 1) | bits[i * 8 + b];
    }
    out[i] = val;
  }

  let padIdx = bits.length / 8;
  let toggle = true;
  while (padIdx < totalDataCodewords) {
    out[padIdx] = toggle ? 0xec : 0x11;
    toggle = !toggle;
    padIdx += 1;
  }
  return out;
}

function interleaveCodewords(data: Uint8Array, spec: QrVersionSpec): Uint8Array {
  const dataBlocks: Uint8Array[] = [];
  const ecBlocks: Uint8Array[] = [];
  let offset = 0;

  for (let i = 0; i < spec.g1Blocks; i += 1) {
    const block = data.subarray(offset, offset + spec.g1Data);
    offset += spec.g1Data;
    dataBlocks.push(block);
    ecBlocks.push(rsComputeRemainder(block, spec.ecPerBlock));
  }
  for (let i = 0; i < spec.g2Blocks; i += 1) {
    const block = data.subarray(offset, offset + spec.g2Data);
    offset += spec.g2Data;
    dataBlocks.push(block);
    ecBlocks.push(rsComputeRemainder(block, spec.ecPerBlock));
  }

  const result = new Uint8Array(spec.totalCodewords);
  let outIdx = 0;
  const maxDataLen = Math.max(spec.g1Data, spec.g2Data);
  for (let i = 0; i < maxDataLen; i += 1) {
    for (const block of dataBlocks) {
      if (i < block.length) {
        result[outIdx] = block[i];
        outIdx += 1;
      }
    }
  }
  for (let i = 0; i < spec.ecPerBlock; i += 1) {
    for (const ec of ecBlocks) {
      result[outIdx] = ec[i];
      outIdx += 1;
    }
  }
  return result;
}

function buildFunctionGrid(spec: QrVersionSpec): {
  size: number;
  modules: boolean[][];
  isFunction: boolean[][];
} {
  const size = spec.version * 4 + 17;
  const modules: boolean[][] = Array.from({ length: size }, () => Array(size).fill(false));
  const isFunction: boolean[][] = Array.from({ length: size }, () => Array(size).fill(false));

  const setFn = (r: number, c: number, dark: boolean) => {
    if (r >= 0 && r < size && c >= 0 && c < size) {
      modules[r][c] = dark;
      isFunction[r][c] = true;
    }
  };

  const drawFinder = (top: number, left: number) => {
    for (let dr = -1; dr <= 7; dr += 1) {
      for (let dc = -1; dc <= 7; dc += 1) {
        const r = top + dr;
        const c = left + dc;
        if (r < 0 || r >= size || c < 0 || c >= size) continue;
        const inOuter = dr >= 0 && dr <= 6 && dc >= 0 && dc <= 6;
        const inRing = dr === 0 || dr === 6 || dc === 0 || dc === 6;
        const inCenter = dr >= 2 && dr <= 4 && dc >= 2 && dc <= 4;
        setFn(r, c, inOuter && (inRing || inCenter));
      }
    }
  };

  drawFinder(0, 0);
  drawFinder(0, size - 7);
  drawFinder(size - 7, 0);

  // Timing patterns
  for (let i = 8; i < size - 8; i += 1) {
    const dark = i % 2 === 0;
    setFn(6, i, dark);
    setFn(i, 6, dark);
  }

  // Alignment patterns (skip corners overlapping the 3 finder + separator regions)
  const centers = spec.alignCenters;
  for (const cr of centers) {
    for (const cc of centers) {
      const overlapsFinder =
        (cr <= 8 && cc <= 8) ||
        (cr <= 8 && cc >= size - 9) ||
        (cr >= size - 9 && cc <= 8);
      if (overlapsFinder) continue;
      for (let dr = -2; dr <= 2; dr += 1) {
        for (let dc = -2; dc <= 2; dc += 1) {
          const dist = Math.max(Math.abs(dr), Math.abs(dc));
          setFn(cr + dr, cc + dc, dist === 0 || dist === 2);
        }
      }
    }
  }

  // Dark module
  setFn(size - 8, 8, true);

  // Reserve format info cells (values populated after mask selection)
  for (let i = 0; i < 9; i += 1) {
    if (i !== 6) {
      setFn(8, i, false);
      setFn(i, 8, false);
    }
  }
  for (let i = size - 8; i < size; i += 1) {
    setFn(8, i, false);
  }
  for (let i = size - 7; i < size; i += 1) {
    setFn(i, 8, false);
  }

  // Version info for v >= 7
  if (spec.version >= 7) {
    const verBits = computeBchVersionInfo(spec.version);
    for (let i = 0; i < 18; i += 1) {
      const bit = ((verBits >> i) & 1) === 1;
      const a = Math.floor(i / 3);
      const b = size - 11 + (i % 3);
      setFn(b, a, bit);
      setFn(a, b, bit);
    }
  }

  return { size, modules, isFunction };
}

function drawFormatBits(modules: boolean[][], size: number, mask: number): void {
  const fmt = computeBchFormatInfo(mask);
  for (let i = 0; i <= 5; i += 1) {
    modules[i][8] = ((fmt >> i) & 1) === 1;
  }
  modules[7][8] = ((fmt >> 6) & 1) === 1;
  modules[8][8] = ((fmt >> 7) & 1) === 1;
  modules[8][7] = ((fmt >> 8) & 1) === 1;
  for (let i = 9; i <= 14; i += 1) {
    modules[8][14 - i] = ((fmt >> i) & 1) === 1;
  }

  for (let i = 0; i <= 7; i += 1) {
    modules[8][size - 1 - i] = ((fmt >> i) & 1) === 1;
  }
  for (let i = 8; i <= 14; i += 1) {
    modules[size - 15 + i][8] = ((fmt >> i) & 1) === 1;
  }
}

export function isQrMaskBitSet(mask: number, r: number, c: number): boolean {
  switch (mask) {
    case 0:
      return (r + c) % 2 === 0;
    case 1:
      return r % 2 === 0;
    case 2:
      return c % 3 === 0;
    case 3:
      return (r + c) % 3 === 0;
    case 4:
      return (Math.floor(r / 2) + Math.floor(c / 3)) % 2 === 0;
    case 5:
      return ((r * c) % 2) + ((r * c) % 3) === 0;
    case 6:
      return (((r * c) % 2) + ((r * c) % 3)) % 2 === 0;
    case 7:
      return (((r + c) % 2) + ((r * c) % 3)) % 2 === 0;
    default:
      return false;
  }
}

function computePenaltyScore(modules: boolean[][], size: number): number {
  let penalty = 0;

  // Rule 1: 5+ consecutive same color in row or col
  for (let r = 0; r < size; r += 1) {
    let runColor = modules[r][0];
    let runLen = 1;
    for (let c = 1; c < size; c += 1) {
      if (modules[r][c] === runColor) {
        runLen += 1;
      } else {
        if (runLen >= 5) penalty += 3 + (runLen - 5);
        runColor = modules[r][c];
        runLen = 1;
      }
    }
    if (runLen >= 5) penalty += 3 + (runLen - 5);
  }
  for (let c = 0; c < size; c += 1) {
    let runColor = modules[0][c];
    let runLen = 1;
    for (let r = 1; r < size; r += 1) {
      if (modules[r][c] === runColor) {
        runLen += 1;
      } else {
        if (runLen >= 5) penalty += 3 + (runLen - 5);
        runColor = modules[r][c];
        runLen = 1;
      }
    }
    if (runLen >= 5) penalty += 3 + (runLen - 5);
  }

  // Rule 2: 2x2 blocks of same color
  for (let r = 0; r < size - 1; r += 1) {
    for (let c = 0; c < size - 1; c += 1) {
      const m = modules[r][c];
      if (modules[r][c + 1] === m && modules[r + 1][c] === m && modules[r + 1][c + 1] === m) {
        penalty += 3;
      }
    }
  }

  // Rule 3: 1:1:3:1:1 finder-like pattern with 4 light modules on either side
  const matchesFinderSeq = (seq: boolean[]) => {
    for (let i = 0; i <= seq.length - 11; i += 1) {
      const p1 =
        seq[i] &&
        !seq[i + 1] &&
        seq[i + 2] &&
        seq[i + 3] &&
        seq[i + 4] &&
        !seq[i + 5] &&
        seq[i + 6] &&
        !seq[i + 7] &&
        !seq[i + 8] &&
        !seq[i + 9] &&
        !seq[i + 10];
      const p2 =
        !seq[i] &&
        !seq[i + 1] &&
        !seq[i + 2] &&
        !seq[i + 3] &&
        seq[i + 4] &&
        !seq[i + 5] &&
        seq[i + 6] &&
        seq[i + 7] &&
        seq[i + 8] &&
        !seq[i + 9] &&
        seq[i + 10];
      if (p1 || p2) penalty += 40;
    }
  };
  for (let r = 0; r < size; r += 1) {
    matchesFinderSeq(modules[r]);
  }
  for (let c = 0; c < size; c += 1) {
    const col: boolean[] = [];
    for (let r = 0; r < size; r += 1) col.push(modules[r][c]);
    matchesFinderSeq(col);
  }

  // Rule 4: dark module proportion balance
  let darkCount = 0;
  for (let r = 0; r < size; r += 1) {
    for (let c = 0; c < size; c += 1) {
      if (modules[r][c]) darkCount += 1;
    }
  }
  const pct = (darkCount * 100) / (size * size);
  const k = Math.min(Math.abs(Math.floor(pct / 5) * 5 - 50), Math.abs(Math.ceil(pct / 5) * 5 - 50)) / 5;
  penalty += k * 10;

  return penalty;
}

export interface EncodedQrMatrix {
  version: number;
  size: number;
  mask: number;
  modules: boolean[][];
  isFunction: boolean[][];
}

/** Encode arbitrary UTF-8 text into a scannable ISO/IEC 18004 Byte-mode QR matrix (ECC Level L). */
export function encodeQrMatrix(text: string): EncodedQrMatrix {
  const bytes = new TextEncoder().encode(text);
  const spec = selectSpecForPayload(bytes.length);
  const dataCodewords = encodeDataCodewords(bytes, spec);
  const finalCodewords = interleaveCodewords(dataCodewords, spec);
  const { size, modules, isFunction } = buildFunctionGrid(spec);

  // Place data bits in zigzag 2-column order
  let bitIdx = 0;
  const totalPayloadBits = finalCodewords.length * 8;
  let upward = true;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let step = 0; step < size; step += 1) {
      const r = upward ? size - 1 - step : step;
      for (let dc = 0; dc < 2; dc += 1) {
        const c = right - dc;
        if (!isFunction[r][c]) {
          let bit = false;
          if (bitIdx < totalPayloadBits) {
            const byteVal = finalCodewords[bitIdx >> 3];
            bit = ((byteVal >> (7 - (bitIdx & 7))) & 1) === 1;
            bitIdx += 1;
          }
          modules[r][c] = bit;
        }
      }
    }
    upward = !upward;
  }

  let bestMask = 0;
  let bestScore = Number.POSITIVE_INFINITY;
  let bestModules = modules;

  for (let mask = 0; mask < 8; mask += 1) {
    const candidate = modules.map((row, r) =>
      row.map((val, c) => (!isFunction[r][c] && isQrMaskBitSet(mask, r, c) ? !val : val))
    );
    drawFormatBits(candidate, size, mask);
    const score = computePenaltyScore(candidate, size);
    if (score < bestScore) {
      bestScore = score;
      bestMask = mask;
      bestModules = candidate;
    }
  }

  return {
    version: spec.version,
    size,
    mask: bestMask,
    modules: bestModules,
    isFunction,
  };
}

export function buildQrSvgPath(matrix: EncodedQrMatrix): string {
  const parts: string[] = [];
  for (let r = 0; r < matrix.size; r += 1) {
    for (let c = 0; c < matrix.size; c += 1) {
      if (matrix.modules[r][c]) {
        parts.push(`M${c} ${r}h1v1h-1z`);
      }
    }
  }
  return parts.join('');
}

export interface QrCodeSvgProps {
  value: string;
  size?: number;
  quietZoneModules?: number;
  className?: string;
  style?: CSSProperties;
  ariaLabel?: string;
}

export function QrCodeSvg({
  value,
  size = 148,
  quietZoneModules = 3,
  className = '',
  style,
  ariaLabel = '同步配对二维码',
}: QrCodeSvgProps) {
  const encoded = encodeQrMatrix(value);
  const totalUnits = encoded.size + quietZoneModules * 2;
  const d = buildQrSvgPath(encoded);

  return createElement(
    'svg',
    {
      role: 'img',
      'aria-label': ariaLabel,
      'data-testid': 'sync-pairing-qr',
      'data-qr-version': encoded.version,
      'data-qr-size': encoded.size,
      width: size,
      height: size,
      viewBox: `-${quietZoneModules} -${quietZoneModules} ${totalUnits} ${totalUnits}`,
      shapeRendering: 'crispEdges',
      className,
      style: {
        backgroundColor: '#ffffff',
        display: 'block',
        ...style,
      },
    },
    createElement('rect', {
      x: -quietZoneModules,
      y: -quietZoneModules,
      width: totalUnits,
      height: totalUnits,
      fill: '#ffffff',
    }),
    createElement('path', { d, fill: '#0b0d12' })
  );
}
