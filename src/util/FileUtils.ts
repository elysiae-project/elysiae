import { invoke } from "@tauri-apps/api/core";
import { appDataDir, basename, join } from "@tauri-apps/api/path";
import { exists, remove } from "./Fs";
import { error, info, warn } from "@tauri-apps/plugin-log";
import { Command } from "@tauri-apps/plugin-shell";
import { executeShellCommand } from "./AppFunctions";
import { rename } from "./Fs";

/**
 * Checks integrity of a file with sha256sum
 * @param file path to a file
 * @param hash __sha256sum__ to verify the file with
 * @returns ``boolean`` condition based on weather the sha256sum of ``file`` matches ``hash``
 */
export const isFileValid = async (
	file: string,
	hash: string,
): Promise<boolean> => {
	return new Promise((resolve) => {
		invoke("get_sha256_sum", {
			file: file,
		}).then((fileHash) => {
			resolve(fileHash === hash);
		});
	});
};

/**
 * @param dir Directory to search
 * @returns string array storing all files found within ``dir`` and any sub-dirs
 */
export const getAllFiles = async (dir: string): Promise<string[]> => {
	return new Promise((resolve, reject) => {
		invoke("get_all_files", {
			path: dir,
		})
			.then((res) => {
				resolve(res as string[]);
			})
			.catch((e) => {
				error(e);
				reject(e);
			});
	});
};

/**
 * @param dir directory to search
 * @returns ``string`` array storing all directories found within dir and any sub-dirs
 */
export const getAllDirs = async (dir: string): Promise<string[]> => {
	return new Promise((resolve, reject) => {
		invoke("get_all_directories", {
			path: dir,
		})
			.then((res) => {
				resolve(res as string[]);
			})
			.catch((e) => {
				error(e);
				reject(e);
			});
	});
};

/**
 * Extracts a compressed archive to a specified location. Supports any archive format that ``7za`` supports
 * @param path Path to archive
 * @param destination destination to extract to
 */
export const extractFile = async (
	path: string,
	destination: string,
): Promise<void> => {
	info(path);
	if (await exists(path)) {
		info(await appDataDir());
		const fullPath = await join(await appDataDir(), path);
		const fullDestination = await join(await appDataDir(), destination);
		await Command.sidecar("binaries/7za", [
			"x",
			fullPath,
			"-sdel",
			`-o${fullDestination}`,
		])
			.execute()
			.catch((e) => {
				console.error(`extractFile: ${e}`);
				error(`extractFile: ${e}`);
				return;
			});

		const tarRegex = /\.tar\.[^.]+$/; // e.g. .tar.gz, .tar.bz2
		if (tarRegex.test(path)) {
			// Use lastIndexOf to correctly strip only the last two extensions
			const baseName = await basename(path);
			const tarName = `${baseName.substring(0, baseName.lastIndexOf(".tar."))}.tar`;

			// Pass relative path — join with destination (relative), not fullDestination (absolute)
			const relativeTarball = await join(destination, tarName);
			await extractFile(relativeTarball, destination);
		}
	} else {
		error(`extractFile: ${path} does not exist`);
	}
};

/**
 * Moves all items at the top level of a directory to a specified location
 * @param itemsDir Initial directory
 * @param newLocation Directory where all items in ``itemsDir`` will be moved to
 * @param removeOriginal Remove original directory (Defaults to ``true``)
 */
export const moveDirItems = async (
	itemsDir: string,
	newLocation: string,
	removeOriginal: boolean = true,
) => {
	const dirs = await getTopLevelFiles(itemsDir);
	for(let i = 0; i < dirs.length; i++) {
		const fileName = await basename(dirs[i]);
		const newPath = await join(newLocation, fileName);
		await rename()
	}

	const appData = await appDataDir();
	const fullItemsDir = await join(appData, itemsDir);
	const fullNewLocation = await join(appData, newLocation);
	info(`mv -v "${fullItemsDir}"/* "${fullNewLocation}"`)
	await executeShellCommand(`mv -v "${fullItemsDir}"/* "${fullNewLocation}"`)

	if (removeOriginal) {
		await remove(itemsDir);
	}
};

/**
 * @param dir directory
 * @returns string array for all files found in the top level of ``dir``
 */
export const getTopLevelFiles = async (dir: string): Promise<string[]> => {
	return new Promise((resolve, reject) => {
		exists(dir).then((locationExists) => {
			if (locationExists) {
				invoke("get_top_level_files", {
					path: dir,
				})
					.then((res) => {
						resolve(res as string[]);
					})
					.catch((e) => {
						reject(`getTopLevelFiles: ${e}`);
					});
			} else {
				reject(`getTopLevelFiles: Location ${dir} does not exist`);
			}
		});
	});
};
