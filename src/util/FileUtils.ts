import { invoke } from "@tauri-apps/api/core";
import { basename, dirname, join } from "@tauri-apps/api/path";
import { exists } from "@tauri-apps/plugin-fs";
import { error } from "@tauri-apps/plugin-log";
import { Command } from "@tauri-apps/plugin-shell";

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
 * @param file Path to archive
 * @param destination destination to extract to
 */
export const extractFile = async (
	file: string,
	destination: string,
): Promise<void> => {
	if (await exists(file)) {
		await Command.sidecar("binaries/7za", [
			"x",
			file,
			"-sdel",
			`-o${destination}` // 7z is weird like this. It only works like this
		])
			.execute()
			.catch((e) => {
				console.error(`extractFile: ${e}`);
				error(`extractFile: ${e}`);
				return;
			});

		const regex = /\.tar\.(xz|bz2|gz|zstd)$/i;
		if (regex.test(file)) {
			// Compressed tarball extracts to dest/filename.tar
			const fileName = `${(await basename(file)).split(".")[0]}.tar`;
			const tarballLocation = await join(destination, fileName);
			await extractFile(tarballLocation, destination);
		}
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
	await Command.create("sh", [
		"-c",
		`mv -v "${itemsDir}"/* "${newLocation}"`,
	]).execute();

	if (removeOriginal) {
		await Command.create("sh", ["-c", `rm -rf "${itemsDir}"`]).execute();
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
