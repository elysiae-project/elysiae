import { invoke } from "@tauri-apps/api/core";
import { appDataDir, basename, join } from "@tauri-apps/api/path";
import { exists, remove } from "./Fs";
import { error, info, warn } from "@tauri-apps/plugin-log";
import { Command } from "@tauri-apps/plugin-shell";
import { executeShellCommand } from "./AppFunctions";
import { rename } from "./Fs";

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

		const tarRegex = /\.tar\.[^.]+$/;
		if (tarRegex.test(path)) {
			const baseName = await basename(path);
			const tarName = `${baseName.substring(0, baseName.lastIndexOf(".tar."))}.tar`;

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
