import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { error } from "@tauri-apps/plugin-log";

/**
 * @param url Link to an API
 * @returns JavaScipt Object from API URL
 */
export const getApiJson = async <T>(url: string): Promise<T> => {
	return new Promise((resolve, reject) => {
		if (!isURLValid(url)) {
			error(`getApiJson: URL ${url} is invalid`);
			reject(`getApiJson: URL ${url} is invalid`);
		}
		fetch(url, {
			method: "GET",
		})
			.then((response) => {
				if (response.status === 200) {
					response
						.json()
						.then((json) => {
							resolve(json as T);
						})
						.catch((e) => {
							error(`getApiJson: ${e}`);
							reject(`getApiJson: ${e}`);
						});
				} else {
					error(`getAPIJson: ${url} returned status code ${response.status}`);
					reject(`getAPIJson: ${url} returned status code ${response.status}`);
				}
			})
			.catch((e) => {
				error(`getApiJson: fetch failed: ${e}`);
				reject(`getApiJson: fetch failed: ${e}`);
			});
	});
};

/**
 * @param verifyingString The string you want to verify
 * @returns Boolean value based on weather verifyingString is a valid http URL
 *   or not
 */
export const isURLValid = (verifyingString: string): boolean => {
	try {
		const testURL = new URL(verifyingString);
		return testURL.protocol === "http:" || testURL.protocol === "https:";
	} catch {
		return false;
	}
};

export const downloadFile = async (
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
		await invoke<void>("download_file", {
			url: url,
			dest: destination,
			uuid: downloadID,
		});
	} catch (e) {
		error(e as string);
		console.log(e);
	} finally {
		unlisten();
	}
};

export const downloadFileNoProgress = async (
	url: string,
	destination: string,
) => {
	const downloadID = crypto.randomUUID();
	await invoke<void>("download_file", {
		url: url,
		dest: destination,
		uuid: downloadID,
	});
};
