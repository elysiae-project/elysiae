import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

export const downloadFile = async (
	url: string,
	destination: string,
	onProgress: (progress: number, total: number) => void,
	broadcastProgress: boolean = true,
) => {
	const downloadID = crypto.randomUUID();

	const unlisten = broadcastProgress
		? await listen<{ progress: number; total: number }>(
				`download://progress/${downloadID}`,
				({ payload }) => {
					onProgress(payload.progress, payload.total);
				},
			)
		: null;

	try {
		await invoke<void>("download_file", {
			url: url,
			dest: destination,
			uuid: downloadID,
		});
	} finally {
		if (unlisten) unlisten();
	}
};
