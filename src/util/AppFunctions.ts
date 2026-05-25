import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { appDataDir, join } from "@tauri-apps/api/path";
import { fetch } from "@tauri-apps/plugin-http";
import { Command } from "@tauri-apps/plugin-shell";
import { type GameCodes, Variants } from "../types";

export const gameCodeToVariant: Record<GameCodes, Variants> = {
	nap: Variants.NAP,
	hkrpg: Variants.HKRPG,
	hk4e: Variants.HK4E,
	bh3: Variants.BH3,
};

export const variantToGameCode: Record<Variants, GameCodes> = {
	0: "bh3",
	1: "hk4e",
	2: "hkrpg",
	3: "nap",
};

export const variantToGameName: Record<Variants, string> = {
	0: "\x48\x6f\x6e\x6b\x61\x69\x20\x49\x6d\x70\x61\x63\x74\x20\x33\x72\x64",
	1: "\x47\x65\x6e\x73\x68\x69\x6e\x20\x49\x6d\x70\x61\x63\x74",
	2: "\x48\x6f\x6e\x6b\x61\x69\x3a\x20\x53\x74\x61\x72\x20\x52\x61\x69\x6c",
	3: "\x5a\x65\x6e\x6c\x65\x73\x73\x20\x5a\x6f\x6e\x65\x20\x5a\x65\x72\x6f",
};

export const variantToExeName: Record<Variants, string> = {
	0: "\x42\x48\x33.exe",
	1: "\x47\x65\x6e\x73\x68\x69\x6e\x49\x6d\x70\x61\x63\x74.exe",
	2: "\x53\x74\x61\x72\x52\x61\x69\x6c.exe",
	3: "\x5a\x65\x6e\x6c\x65\x73\x73\x5a\x6f\x6e\x65\x5a\x65\x72\x6f.exe",
};

export const getGameSize = async (game: Variants): Promise<number> => {
	return new Promise((resolve, reject) => {
		join("games", variantToGameCode[game])
			.then((gameDir) => {
				invoke("get_dir_size", {
					path: gameDir,
				})
					.then((res) => {
						resolve((res as number) / 1024 ** 3);
					})
					.catch(reject);
			})
			.catch(reject);
	});
};

/**
 * @returns value based on weather or not the app is running in a development environment
 */
export const inDevEnv = async (): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		invoke<boolean>("in_dev_env").then(resolve).catch(reject);
	});
};

/**
 * Executes a command on the shell
 *
 * @param command Any command
 * @param env (optional) environment variables
 */
export const executeShellCommand = async (
	command: string,
	env?: Record<string, string> | undefined,
): Promise<void> => {
	await Command.create("sh", ["-c", command], {
		env: env,
	}).execute();
};

/**
 * Executes a command of a binary found in the Elysiae's app data directory
 *
 * @param binaryPath Path to binary, relative to the app data directory
 * @param args Arguments to pass into command
 * @param env (optional) environment variables
 */
export const executeLocalBinary = async (
	binaryPath: string,
	args?: string,
	env?: Record<string, string> | undefined,
): Promise<void> => {
	const appData = await appDataDir();
	await executeShellCommand(
		`${appData}/${binaryPath} ${typeof args !== "undefined" ? args : ""}`,
		env,
	).catch((e) => {
		throw new Error(e);
	});
};

export const formatNumber = (num: number): string => {
	try {
		return new Intl.NumberFormat(navigator.language).format(num);
	} catch {
		return new Intl.NumberFormat("en-US").format(num);
	}
};

/**
 * @param url Link to an API
 * @returns JavaScipt Object from API URL
 */
export const getApiJson = async <T>(url: string): Promise<T> => {
	return new Promise((resolve, reject) => {
		if (!isURLValid(url)) {
			reject(`getApiJson: URL ${url} is invalid`);
		}
		fetch(url, {
			method: "GET",
		}).then((response) => {
			if (response.status === 200) {
				response
					.json()
					.then((json) => {
						resolve(json as T);
					})
					.catch((e) => {
						reject(`getApiJson: ${e}`);
					});
			} else {
				reject(`getAPIJson: ${url} returned status code ${response.status}`);
			}
		});
	});
};

/**
 * @param verifyingString The string you want to verify
 * @returns Boolean value based on weather verifyingString is a valid http URL
 *   or not
 */
const isURLValid = (verifyingString: string): boolean => {
	try {
		const testURL = new URL(verifyingString);
		return testURL.protocol === "http:" || testURL.protocol === "https:";
	} catch {
		return false;
	}
};

export const downloadFileWithProgress = async (
	url: string,
	destination: string,
	onProgress: (progress: number, total: number) => void,
) => {
	const downloadID = crypto.randomUUID();

	const unlisten = await listen<{ progress: number; total: number }>(
		`download://progress/${downloadID}`,
		({ payload }) => {
			onProgress(payload.progress, payload.total);
		},
	);

	try {
		await invoke("download_file", {
			url: url,
			dest: destination,
			uuid: downloadID,
		});
	} finally {
		unlisten();
	}
};
