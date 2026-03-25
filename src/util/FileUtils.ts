import { invoke } from "@tauri-apps/api/core";
import { exists, remove } from "../lib/Fs";
import { error, info } from "@tauri-apps/plugin-log";
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
 * @param archivePath Path to archive
 * @param dest destination to extract to
 */
export const extractFile = async (
	archivePath: string,
	dest: string,
): Promise<void> => {
	info(archivePath);
	if (await exists(archivePath)) {
		await invoke("extract_file", {
			archive: archivePath,
			dest: dest,
		});
		remove(archivePath);
	} else {
		error(`extractFile: ${archivePath} does not exist`);
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
