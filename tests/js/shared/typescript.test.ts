import test from "vael:test";
import assert from "vael:assert";
import { equalFold, trim } from "vael:strings";
import { v4, v7 } from "vael:uuid";
import type { NeverLoaded } from "./never-loaded-at-runtime.d.ts";
import type * as StringsModule from "vael:strings";

interface Identifiable {
    readonly id: string;
}

interface TaggedRecord<T> extends Identifiable {
    label: string;
    payload: T;
    meta?: NeverLoaded;
}

type Pair<A, B> = [A, B];
type TrimMode = "both" | "start";

class Collector<T> {
    #items: T[] = [];
    readonly name: string;

    constructor(name: string) {
        this.name = name;
    }

    push(item: T): number {
        return this.#items.push(item);
    }

    snapshot(): Pair<string, T[]> {
        return [this.name, [...this.#items]];
    }
}

function identity<T>(value: T): T {
    return value;
}

const pairWith = <A, B>(left: A, right: B): Pair<A, B> => [left, right];

function formatTag(value: string): string;
function formatTag(value: number): string;
function formatTag(value: string | number): string {
    return typeof value === "number" ? `n:${value}` : `s:${value}`;
}

function normalizeLabel(raw: string, mode: TrimMode = "both", chars?: string): string {
    if (mode === "start") {
        return raw.trimStart();
    }
    return chars === undefined ? trim(raw) : trim(raw, chars);
}

async function buildTaggedRecord<T>(label: string, payload: T): Promise<TaggedRecord<T>> {
    const id: string = await Promise.resolve(v4());
    const cleanLabel: string = normalizeLabel(label, "both", ".");
    return { id, label: cleanLabel, payload } satisfies TaggedRecord<T>;
}

async function verifyUuidAsync(version: 4 | 7): Promise<string> {
    const generated: string = await Promise.resolve(version === 4 ? v4() : v7());
    return generated;
}

test("type annotations, interfaces, type aliases, generics, and import type are erased", () => {
    const collector = new Collector<number>("counts");
    assert.strictEquals(collector.push(10), 1);
    assert.strictEquals(collector.push(20), 2);

    const [name, items]: Pair<string, number[]> = collector.snapshot();
    assert.strictEquals(name, "counts");
    assert.equals(items, [10, 20]);

    const foldedCheck: typeof StringsModule.equalFold = equalFold;
    assert.strictEquals(foldedCheck(identity("VaelJS"), "vaeljs"), true);

    const combined = pairWith(formatTag("ok"), formatTag(7));
    assert.equals(combined, ["s:ok", "n:7"]);
});

test("as, satisfies, non-null assertions, and runtime non-checking preserve values", () => {
    const config = {
        mode: "both" as TrimMode,
        chars: "_",
    } as const satisfies { mode: TrimMode; chars: string };

    const cleaned = normalizeLabel("__hello__", config.mode, config.chars);
    assert.strictEquals(cleaned, "hello");

    const maybeText: string | undefined = "K";
    const guaranteed: string = maybeText!;
    assert.strictEquals(equalFold(guaranteed, "\u212a"), true);

    // Stripping never type-checks: runtime values flow through unchanged.
    const mismatched: number = "runtime-string" as unknown as number;
    assert.strictEquals(typeof mismatched, "string");
    assert.strictEquals(mismatched as unknown as string, "runtime-string");
});

test("typed async functions interoperate with vael:uuid, vael:strings, and vael:assert", async () => {
    const record: TaggedRecord<Pair<number, string>> = await buildTaggedRecord(
        "..alpha..",
        pairWith(1, "one"),
    );

    const uuidV4Pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
    const uuidV7Pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

    assert.ok(uuidV4Pattern.test(record.id), `expected v4 uuid, got ${record.id}`);
    assert.strictEquals(record.label, "alpha");
    assert.equals(record.payload, [1, "one"]);

    const seven: string = await verifyUuidAsync(7);
    assert.ok(uuidV7Pattern.test(seven), `expected v7 uuid, got ${seven}`);

    const resolved: string = await assert.doesNotReject(() => verifyUuidAsync(4));
    assert.ok(uuidV4Pattern.test(resolved));

    const rejection = await assert.rejects(async (): Promise<never> => {
        throw new RangeError("typed async failure");
    }, RangeError);
    assert.strictEquals(rejection.message, "typed async failure");
});

test("type erasure preserves line numbers in Error stack traces", () => {
    const throwOnKnownLine = (tag: string): never => {
        throw new Error(`trace:${tag}`);
    };
    const caught = assert.throws(() => throwOnKnownLine("pos"), Error);
    const stack: string = caught.stack ?? "";
    assert.ok(stack.includes("typescript.test.ts:130"), `unexpected stack: ${stack}`);
});
