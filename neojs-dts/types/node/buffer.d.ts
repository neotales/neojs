declare module "node:buffer" {
  export type BufferEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";

  export class Buffer extends Uint8Array {
    static from(value: string, encoding?: BufferEncoding): Buffer;
    static from(value: ArrayBuffer | SharedArrayBuffer, byteOffset?: number, length?: number): Buffer;
    static from(value: ArrayLike<number> | ArrayBufferView): Buffer;
    static alloc(size: number, fill?: string | number | Uint8Array, encoding?: BufferEncoding): Buffer;
    static allocUnsafe(size: number): Buffer;
    static concat(list: readonly Uint8Array[], totalLength?: number): Buffer;
    static isBuffer(value: unknown): value is Buffer;
    static byteLength(value: string | ArrayBufferView | ArrayBuffer | SharedArrayBuffer, encoding?: BufferEncoding): number;

    toString(encoding?: BufferEncoding, start?: number, end?: number): string;
    toJSON(): { type: "Buffer"; data: number[] };
    equals(otherBuffer: Uint8Array): boolean;
  }

  const _default: {
    Buffer: typeof Buffer;
  };
  export default _default;
}
