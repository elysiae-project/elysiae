import { listen } from "@tauri-apps/api/event";
import { Download } from "../types";
import { error, info } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";
import { CustomEventManager } from "./CustomEventManager";
import { exists } from "@tauri-apps/plugin-fs";

export const downloads = new Map<string, Download>();
export const downloadEvent = new CustomEventManager();

/**
 * Downloads file(s) to a specified location and adds currently active downloads to the ``downloads`` Map()
 * @param links Link(s) to download
 * @param destination destination to save links to
 */
export const downloadFile = async (
	links: string | string[],
	destination: string,
): Promise<void> => {
	if (typeof links == "string") {
		// If typescript were to be able to do single-length strings implicitly... sigh....
		return await downloadFile([links], destination);
	}
	const activeDownloads: Promise<any>[] = [];
	const uuids = [];

	try {
		for (let i = 0; i < (links as string[]).length; i++) {
			uuids.push(crypto.randomUUID());
			if (!(await exists(destination))) {
				activeDownloads.push(
					download((links as string[])[i], destination, uuids[i]),
				);
			}
            else {
                info(`Download Skipped as file already exists`);
                continue;
            };
		}
		await Promise.all(activeDownloads).catch((e) => {
			error(`downloadFile: ${e}`);
		});
	} finally {
		for (let i = 0; i < uuids.length; i++) {
			downloads.delete(uuids[i]);
		}
	}
};

const download = async (
	url: string,
	destination: string,
	uuid: string,
): Promise<void> => {
	const unlisten = await listen<{ progress: number; total: number }>(
		`download://progress`,
		({ payload }) => {
			const status: Download = {
				// Progress/total is stored as bytes. Convert to Megabytes
				// If other functions want to convert to other units (Kilo, Giga, Tera, etc.) It will look much cleaner on their end
				downloaded: payload.progress / 1024 ** 2,
				total: payload.total / 1024 ** 2,
			};
			// info(`Downloaded ${status.downloaded.toFixed(2)} of ${status.total.toFixed(2)}Mb`,);
			downloads.set(uuid, status);
			downloadEvent.dispatchEvent("downloadChanged", { uuid, status });
		},
	);

	try {
		await invoke("download_file", {
			downloadUrl: url,
			destination: destination,
		}).catch((e) => {
			console.error(`downloadFile: ${e}`);
			unlisten();
		});
	} finally {
		unlisten();
	}
};
