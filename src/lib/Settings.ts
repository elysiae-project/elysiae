import { appDataDir, join } from "@tauri-apps/api/path";
import { error } from "@tauri-apps/plugin-log";
import { load, type Store } from "@tauri-apps/plugin-store";
import type { AppOptions } from "../types";

let storePromise: Promise<Store> | undefined;
let store: Store | undefined;

const loadStore = (): Promise<Store> => {
	if (!storePromise) {
		storePromise = (async () => {
			const appData = await appDataDir();
			const storePath = await join(appData, "settings.json");
			return load(storePath);
		})();
	}
	return storePromise;
};

export const getOption = async <T = unknown>(
	key: AppOptions,
): Promise<T | undefined> => {
	if (!store) {
		store = await loadStore();
	}
	return store?.get<T>(key);
};

export const setOption = async <T = unknown>(
	key: AppOptions,
	value: T,
): Promise<void> => {
	if (!store) {
		store = await loadStore();
	}
	await store?.set(key, value);
	await store.save();
	await store.reload();
};
