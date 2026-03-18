import { error, info, warn } from "@tauri-apps/plugin-log";
import { isURLValid } from "./WebUtils";
import { multiDownload, singleDownload } from "./DownloadManager";
import { appDataDir, dirname, join } from "@tauri-apps/api/path";
import { extractFile } from "./FileUtils";
import { exists, remove } from "./Fs";
import { GamePkg, Variants } from "../types";
import { getActiveGameCode, getGameExeName } from "./AppFunctions";
import { wineCommand } from "./WineTools";

/**
 * Starts a fresh download for the current active game
 * @param downloadLinks all download URLs for game
 * @param destPath Where all files will be downloaded/extracted to
 */
export const downloadGame = async (
	downloadAsset: GamePkg[],
	destPath: string,
) => {
	// TODO: Add retry download
	const downloadLinks: string[] = [];
	for (let i = 0; i < downloadAsset.length; i++) {
		// TODO: Also get md5 (for file verification) and download size (if needed)
		downloadLinks.push(downloadAsset[i].url);
	}
	info(downloadLinks.toString());

	let destFiles: string[] = [];
	await multiDownload(downloadLinks, await appDataDir());
	for (let i = 0; i < downloadLinks.length; i++) {
		const fileName = downloadAsset[i].url.split("/").pop() as string;
		const temporaryLocation = await join(await appDataDir(), fileName);
		destFiles.push(temporaryLocation);
	}
	
	for (let i = 0; i < destFiles.length; i++) {
		const file = destFiles[i];
		info(file);

		await extractFile(file, destPath);
		await remove(file);
	}
};

export const launchGame = async (gameCode: Variants) => {
	const appData = await appDataDir();
	const jadeite = await join(appData, "jadeite", "jadeite.exe");
	const currentGame = await join(
		appData,
		getActiveGameCode(gameCode),
		getGameExeName(gameCode),
	);

	if (gameCode === Variants.SR || gameCode === Variants.BH) {
		await wineCommand(`${jadeite} ${currentGame}`);
	}
	else {
		await wineCommand(`${currentGame}`);
	}
};

export const cancelDownload = async () => {};

export const isGameInstalled = async (gameCode: Variants): Promise<boolean> => {
	const appData = await appDataDir();
	const currentGame = await join(
		appData,
		getActiveGameCode(gameCode),
		getGameExeName(gameCode),
	);
	return await exists(currentGame);
};

/**
 *
 * @returns ``boolean`` condition based on if the current selected game in the frontend has a preinstallation publicly available
 */
export const isPreinstallAvailable = async (): Promise<boolean> => {
	return true;
};

/**
 * @description Downloads preinstall/update package
 */
export const downloadUpdate = async (
	gameCode: Variants,
	isPreinstall: boolean = false,
) => {
	const updateLink = "";
	const appData = await appDataDir();
	if (isURLValid(updateLink)) {
		const file = updateLink.split("/").pop() as string;
		const fileLocation = await join(appData, file);
		if (!(await exists(fileLocation))) {
			await singleDownload(updateLink, fileLocation);
		}
		if (!isPreinstall) {
			await applyUpdate(fileLocation);
		}
	}
};

/**
 * Unpacks update package
 */
export const applyUpdate = async (updateArchive: string) => {
	const path = await dirname(updateArchive);
	await extractFile(updateArchive, path);
};
