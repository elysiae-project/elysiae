import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { info } from "@tauri-apps/plugin-log";
import { fetch } from "@tauri-apps/plugin-http";

/**
 * @param verifyingString the string you want to verify
 * @returns boolean value based on weather verifyingString is a valid http URL or not
 */
export const isURLValid = (verifyingString: string): boolean => {
	try {
		const testURL = new URL(verifyingString);
		return testURL.protocol === "http:" || testURL.protocol === "https:";
	} catch {
		return false;
	}
};

/**
 * @param url link to an API
 * @returns JavaScipt Object from API URL
 */
export const getApiJson = async (url: string): Promise<any> => {
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
						resolve(json);
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

export const downloadFile = async (url: string, destination: string) => {
	const downloadID = crypto.randomUUID();

	const unlisten = await listen<{ progress: number; total: number }>(
		`download://progress/${downloadID}`,
		({ payload }) => {
			const downloaded = (payload.progress / 1024 ** 2).toFixed(2);
			const total = (payload.total / 1024 ** 2).toFixed(2);
			info(`Downloaded ${downloaded}MB of ${total}MB`);
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
