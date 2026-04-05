import { resolveResource } from "@tauri-apps/api/path";
import { error } from "@tauri-apps/plugin-log";
import { load, Store } from "@tauri-apps/plugin-store";

type Options = "selectedGame" | "voLanguage" | "";
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
			});
	});
};

export const getSettingValue = async (key: Options): Promise<any> => {
	if (!store) {
		store = await loadStore();
	}

	return new Promise((resolve, reject) => {
		store
			?.get<{ value: any }>(key)
			.then((res) => {
				resolve(res);
			})
			.catch((e) => {
				reject(`getOption: ${e}`);
			});
	});
};

export const setOption = async (key: Options, value: any): Promise<void> => {
	if (!store) {
		store = await loadStore();
	}
	await store?.set(key, value);
	await store.save();
};
