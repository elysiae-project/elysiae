import { resolveResource } from "@tauri-apps/api/path";
import { error } from "@tauri-apps/plugin-log";
import { load, type Store } from "@tauri-apps/plugin-store";
import type { Settings } from "../types";

let store: Store | undefined;

const loadStore = async (): Promise<Store> => {
	return new Promise((resolve, reject) => {
		resolveResource("settings.json")
			.then((storeFile) => {
				load(storeFile)
					.then((newStore) => {
						resolve(newStore);
					})
					.catch((e) => {
						error(`loadStore: ${e}`);
						reject(e);
					});
			})
			.catch((e) => {
				error(`loadStore: ${e}`);
				reject(e);
			});
	});
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
