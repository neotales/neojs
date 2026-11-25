declare module "node:crypto" {
  export type DigestEncoding = "hex" | "base64" | "base64url" | "latin1" | "binary";
  export type BinaryLike = string | Uint8Array | ArrayBufferView;

  export interface Hash {
    update(data: BinaryLike, inputEncoding?: string): this;
    digest(): Uint8Array;
    digest(encoding: DigestEncoding): string;
  }

  export interface Hmac {
    update(data: BinaryLike, inputEncoding?: string): this;
    digest(): Uint8Array;
    digest(encoding: DigestEncoding): string;
  }

  export function randomBytes(size: number): Uint8Array;
  export function randomUUID(): string;
  export function createHash(algorithm: string): Hash;
  export function createHmac(algorithm: string, key: BinaryLike): Hmac;
  export function pbkdf2Sync(
    password: BinaryLike,
    salt: BinaryLike,
    iterations: number,
    keylen: number,
    digest: string,
  ): Uint8Array;
  export function timingSafeEqual(a: ArrayBufferView, b: ArrayBufferView): boolean;

  const _default: {
    randomBytes: typeof randomBytes;
    randomUUID: typeof randomUUID;
    createHash: typeof createHash;
    createHmac: typeof createHmac;
    pbkdf2Sync: typeof pbkdf2Sync;
    timingSafeEqual: typeof timingSafeEqual;
  };
  export default _default;
}
