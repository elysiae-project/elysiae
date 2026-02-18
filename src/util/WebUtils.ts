import { error, info } from "@tauri-apps/plugin-log";
import { download } from "@tauri-apps/plugin-upload";
import { isFileValid } from "./FileUtils";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

/**
 * @param url The URL to check
 * @returns HTTP status code of the URL
 */
export const getHttpStatus = async (url: string): Promise<number> => {
	console.log(`Checking HTTP Status of ${url}`);
	return new Promise((resolve, reject) => {
		if (!isURLValid(url)) {
			const message = `The URL ${url} is not valid!`;
			error(message);
			reject(404); // URL isn't valid = not found.
		}
		getRedirectURL(url).then((resolvedURL) => {
			fetch(resolvedURL, {
				method: "GET",
			}).then((response) => {
				resolve(response.status);
			});
		});
	});
};

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
 * @param url Location of the file that is going to be downloaded
 * @param destination Location the file will be saved to
 */
export const downloadFile = async (
	url: string,
	destination: string,
): Promise<void> => {
	console.log(`Attempting to download ${url}`);
	const unlisten = await listen<{ progress: number; total: number }>(
		"download://progress",
		({ payload }) => {
			const percentage = ((payload.progress / payload.total) * 100).toFixed(2);
			console.log(`Downloaded ${payload.progress} of ${payload.total} bytes`);
		},
	);

	try {
		await invoke("download_file", {
			downloadUrl: url,
			destination: destination,
		});
	} finally {
		unlisten();
	}
};

export const getRedirectURL = async (url: string): Promise<string> => {
	console.log(`Getting Redirect URL For: ${url}`);
	return new Promise((resolve, reject) => {
		fetch(url, {
			method: "HEAD",
		})
			.then((res) => {
				console.log("Success!");
				console.log(`Resolved URL: ${res.url}`);
				resolve(res.url as string);
			})
			.catch((err) => {
				console.log("FAIL!!!!");
				reject(err);
			});
	});
};

export const getApiJson = async (url: string): Promise<any> => {
	return new Promise((resolve, reject) => {
		getRedirectURL(url).then((resolvedURL) => {
			fetch(resolvedURL, {
				method: "GET",
			}).then((response) => {
				if (response.status === 200) {
					response.json().then((json) => {
						resolve(json);
					});
				} else {
					const message = `${url} returned status code ${response.status}!`;
					error(message);
					reject(message);
				}
			});
		});
	});
};
