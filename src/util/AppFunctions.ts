import { getCurrentWindow } from "@tauri-apps/api/window";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { fetch } from "@tauri-apps/plugin-http";
import { error, info } from "@tauri-apps/plugin-log";
import { download } from "@tauri-apps/plugin-upload";

export const closeApp = (): void => {
	getCurrentWindow().close();
};

export const minimizeApp = (): void => {
	getCurrentWindow().minimize();
};

export const getActiveGameCode = (): "bh" | "ys" | "sr" | "nap" => {
	switch (useGame()) {
		case Variants.BH:
			return "bh";
		case Variants.YS:
			return "ys";
		case Variants.SR:
			return "sr";
		case Variants.NAP:
			return "nap";
	}
};

export const setActiveGame = async (game: Variants): Promise<void> => {};

/**
 * @param url Location of the file that is going to be downloaded
 * @param destination Location the file will be saved to
 */
export const downloadFile = async (
	url: string,
	destination: string,
): Promise<void> => {
	if ((await getHttpStatus(url)) === 200) {
		download(url, destination, ({ progress, total }) =>
			// Dividing by 1,000,000 Converts the Bytes to Megabytes
			info(
				`Downloaded ${(progress / 1_000_000).toFixed(2)} of ${(total / 1_000_000).toFixed(2)} Mb (${(progress / total).toFixed(2)}%)`,
			),
		);
	} else {
		return;
	}
};

/**
 * @param url The URL to check
 * @returns HTTP status code of the URL
 */
export const getHttpStatus = async (url: string): Promise<number> => {
	if (!isURLValid(url)) {
		const message = `The URL ${url} is not valid!`;
		error(message);
		return 404; // URL isn't valid = not found. 
	}
	const response = await fetch(url, {
		method: "GET",
	});
	const status = response.status;
	return status;
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
