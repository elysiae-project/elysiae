import { warn } from "@tauri-apps/plugin-log";
import { downloadFile, isURLValid } from "./WebUtils";
import { dirname, join, resourceDir } from "@tauri-apps/api/path";
import { extractFile } from "./FileUtils";
import { exists, remove } from "@tauri-apps/plugin-fs";

/**
 * @description Starts a fresh download for the current active game (in the frontend)
 * @param downloadLinks all download URLs for game
 * @param destPath Where all files will be downloaded/extracted to
 */
export const downloadGame = async (
	downloadLinks: string[],
	destPath: string,
) => {
	let destFiles: string[] = [];
	for (let i = 0; i < downloadLinks.length; i++) {
		if (!isURLValid(downloadLinks[i])) {
			warn(`downloadGame: ${downloadLinks[i]} is not a valid URL`);
			continue;
		}

		const fileName = downloadLinks[i].split("/").pop() as string;
		const fileLocation = await join(destPath, fileName);
		destFiles.push(fileLocation);

		await downloadFile(downloadLinks[i], fileLocation);
	}

	for (let i = 0; i < destFiles.length; i++) {
		const file = destFiles[i];
		await extractFile(file, destPath);
		await remove(file);
	}
};

export const isGameInstalled = async(): Promise<boolean> => {
	return new Promise((resolve, reject) => {

	})
}

/**
 * @description Verifies file integrity of all files in a game install
 */
export const verifyInstall = async () => {};

/**
 *
 * @returns ``boolean`` condition based on if the current selected game in the frontend has a preinstallation publicly available
 */
export const isPreinstallAvailable = async (): Promise<boolean> => {
	return new Promise((resolve, reject) => {});
};

/**
 * @description Downloads preinstall/update package
 */
export const downloadUpdate = async (updateLink: string, isPreinstall: boolean = false) => {
	const appDir = await resourceDir();
	if(isURLValid(updateLink)) {
		const file = updateLink.split("/").pop() as string
		const fileLocation = await join(appDir, file);
		if(!(await exists(fileLocation))) {
			await downloadFile(updateLink, fileLocation);
		}
		if (!isPreinstall) {
			await applyUpdate(fileLocation);
		}
	}

};

/**
 * @description unpacks update package
 */
export const applyUpdate = async (updateArchive: string) => {
	const path = await dirname(updateArchive);
	await extractFile(updateArchive, path);
};
