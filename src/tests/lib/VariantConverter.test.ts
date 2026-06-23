import { expect, test } from "vitest";
import {
	gameCodeToVariant,
	variantToExeName,
	variantToGameCode,
	variantToGameName,
} from "../../lib/VariantConverter";
import { type GameCodes, Variants } from "../../types";

test("Convert from game code to variant", () => {
	expect(gameCodeToVariant.bh3).toBe(Variants.BH3);
	expect(gameCodeToVariant.hk4e).toBe(Variants.HK4E);
	expect(gameCodeToVariant.hkrpg).toBe(Variants.HKRPG);
	expect(gameCodeToVariant.nap).toBe(Variants.NAP);
});

test("Convert from Variant to game code", () => {
	expect(variantToGameCode[Variants.BH3]).toBe<GameCodes>("bh3");
	expect(variantToGameCode[Variants.HK4E]).toBe<GameCodes>("hk4e");
	expect(variantToGameCode[Variants.HKRPG]).toBe<GameCodes>("hkrpg");
	expect(variantToGameCode[Variants.NAP]).toBe<GameCodes>("nap");
});

test("Convert from Variant to exe name", () => {
	expect(variantToExeName[Variants.BH3]).toBe("\x42\x48\x33\x2e\x65\x78\x65");
	expect(variantToExeName[Variants.HK4E]).toBe(
		"\x47\x65\x6e\x73\x68\x69\x6e\x49\x6d\x70\x61\x63\x74\x2e\x65\x78\x65",
	);
	expect(variantToExeName[Variants.HKRPG]).toBe(
		"\x53\x74\x61\x72\x52\x61\x69\x6c\x2e\x65\x78\x65",
	);
	expect(variantToExeName[Variants.NAP]).toBe(
		"\x5a\x65\x6e\x6c\x65\x73\x73\x5a\x6f\x6e\x65\x5a\x65\x72\x6f\x2e\x65\x78\x65",
	);
});

test("Convert from Variant to game name", () => {
	expect(variantToGameName[Variants.BH3]).toBe(
		"\x48\x6f\x6e\x6b\x61\x69\x20\x49\x6d\x70\x61\x63\x74\x20\x33\x72\x64",
	);
	expect(variantToGameName[Variants.HK4E]).toBe(
		"\x47\x65\x6e\x73\x68\x69\x6e\x20\x49\x6d\x70\x61\x63\x74",
	);
	expect(variantToGameName[Variants.HKRPG]).toBe(
		"\x48\x6f\x6e\x6b\x61\x69\x3a\x20\x53\x74\x61\x72\x20\x52\x61\x69\x6c",
	);
	expect(variantToGameName[Variants.NAP]).toBe(
		"\x5a\x65\x6e\x6c\x65\x73\x73\x20\x5a\x6f\x6e\x65\x20\x5a\x65\x72\x6f",
	);
});
