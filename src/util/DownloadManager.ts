import { listen } from "@tauri-apps/api/event";
import { Download } from "../types";
import { error, info } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";
import { CustomEventManager } from "./CustomEventManager";
import { exists } from "./Fs";
import { join } from "@tauri-apps/api/path";

export const downloads = new Map<string, Download>();
export const downloadEvent = new CustomEventManager();

/**
 * Downloads file(s) to a specified location and adds currently active downloads to the ``downloads`` Map()
 * @param links Link(s) to download
 * @param destination destination to save links to
 */
export const multiDownload = async (
	links: string[],
	destination: string,
): Promise<void> => {
	info(`Download added: ${links}`);
	const activeDownloads: Promise<any>[] = [];
	const uuids = links.map(() => crypto.randomUUID());

	try {
		for (let i = 0; i < links.length; i++) {
			const fileName = links[i].split("/").pop() as string;
			const downloadLocation = await join(destination, fileName);

			if (!(await exists(downloadLocation))) {
				activeDownloads.push(
					singleDownload(links[i], downloadLocation, uuids[i], false),
				);
			} else {
				info(`Download Skipped as file already exists`);
				continue;
			}
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

export const singleDownload = async (
	url: string,
	destination: string,
	uuid: string = crypto.randomUUID(),
	removeOnComplete: boolean = true,
): Promise<void> => {
	const unlisten = await listen<{ progress: number; total: number }>(
		`download://progress/${uuid}`,
		({ payload }) => {
			const status: Download = {
				// Progress/total is stored as bytes. Convert to Megabytes
				// If other functions want to convert to other units (Kilo, Giga, Tera, etc.) It will look much cleaner on their end
				downloaded: payload.progress / 1024 ** 2,
				total: payload.total / 1024 ** 2,
			};
			info(
				`Downloaded ${status.downloaded.toFixed(2)} of ${status.total.toFixed(2)}Mb`,
			);
			downloads.set(uuid, status);
			downloadEvent.dispatchEvent("downloadChanged", { uuid, status });
		},
	);

	try {
		await invoke("download_file", {
			downloadUrl: url,
			destination: destination,
			uuid: uuid,
		}).catch((e) => {
			console.error(`downloadFile: ${e}`);
			unlisten();
		});
	} finally {
		unlisten();
		if(removeOnComplete) {
			downloads.delete(uuid);
		}
	}
};
