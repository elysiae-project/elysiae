import { appDataDir, join } from "@tauri-apps/api/path";
import { load, type Store } from "@tauri-apps/plugin-store";
import { type Settings, Variants } from "../types";
import { exists, readTextFile, writeTextFile } from "./Fs";

let store: Store | undefined;
const CURRENT_DATA_VERSION = 1;
const DEFAULT_SETTINGS_DATA: Settings = {
	version: CURRENT_DATA_VERSION,
	isFirstLaunch: true,
	lastUsedVersion: "0.0",
	selectedGame: "hk4e",
	voLanguage: "en",
	blockNotifications: false,
	createShortcuts: true,
	autoUpdate: true,
	autoPreload: true,
	installedComponents: {
		proton: null,
		jadeite: null,
	},
	cachedBackgrounds: {
		[Variants.BH3]: [],
		[Variants.HK4E]: [],
		[Variants.HKRPG]: [],
		[Variants.NAP]: [],
	},
};

const loadStore = async (): Promise<Store> => {
	const settingsPath: string = await join("conf", "settings.json");
	if (!(await exists(settingsPath))) {
		await writeTextFile(settingsPath, JSON.stringify(DEFAULT_SETTINGS_DATA));
	}

	await migrateSettings();

	// load() doesn't have any settings for a relative app, so an absolute path must be used instead
	return await load(await join(await appDataDir(), settingsPath));
};

const migrateSettings = async () => {
	const settingsPath: string = await join("conf", "settings.json");
	const settingsData: Settings = JSON.parse(await readTextFile(settingsPath));

	for (let i = settingsData.version; i < CURRENT_DATA_VERSION; i++) {
		switch (i) {
			// Currently unused; there are no migration functions needed in Elysiae yet
		}
		settingsData.version = i + 1;
	}

	// Merge properties and write back to the settings file
	await writeTextFile(
		settingsPath,
		JSON.stringify({ settingsData, ...DEFAULT_SETTINGS_DATA }),
	);
};

export const getOption = async <T = unknown>(
	key: keyof Settings,
): Promise<T | undefined> => {
	if (!store) {
		store = await loadStore();
	}
	return store?.get<T>(key);
};

export const setOption = async <T = unknown>(
	key: keyof Settings,
	value: T,
): Promise<void> => {
	if (!store) {
		store = await loadStore();
	}
	await store?.set(key, value);
	await store.save();
	await store.reload();
};
