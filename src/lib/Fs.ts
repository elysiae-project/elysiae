import { invoke } from "@tauri-apps/api/core";
import {
	BaseDirectory,
	type DirEntry,
	exists as tauriExists,
	mkdir as tauriMkdir,
	readDir as tauriReadDir,
	readFile as tauriReadFile,
	readTextFile as tauriReadTextFile,
	remove as tauriRemove,
	rename as tauriRename,
} from "@tauri-apps/plugin-fs";
import { error, info } from "@tauri-apps/plugin-log";

export const exists = async (path: string): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		tauriExists(path, { baseDir: BaseDirectory.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const readFile = async (
	path: string,
): Promise<Uint8Array<ArrayBuffer>> => {
	return new Promise((resolve, reject) => {
		tauriReadFile(path, { baseDir: BaseDirectory.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const readTextFile = async (path: string): Promise<string> => {
	return new Promise((resolve, reject) => {
		tauriReadTextFile(path, { baseDir: BaseDirectory.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const remove = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, { baseDir: BaseDirectory.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const removeDir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, { recursive: true, baseDir: BaseDirectory.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const mkdir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriMkdir(path, { baseDir: BaseDirectory.AppData, recursive: true })
			.then(resolve)
			.catch(reject);
	});
};

export const rename = async (
	originalPath: string,
	destPath: string,
): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRename(originalPath, destPath, {
			newPathBaseDir: BaseDirectory.AppData,
			oldPathBaseDir: BaseDirectory.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const readDir = async (path: string): Promise<DirEntry[]> => {
	return new Promise((resolve, reject) => {
		tauriReadDir(path, { baseDir: BaseDirectory.AppData })
			.then((res) => {
				resolve(res);
			})
			.catch(reject);
	});
};

/**
 * Extracts a compressed archive to a specified location. Supports most common
 * tar compression formats (gz, xz, zstd) and zip
 *
 * @param archivePath Path to archive
 * @param dest Destination to extract to
 */
export const extractFile = async (
	archivePath: string,
	dest: string,
): Promise<void> => {
	info(archivePath);
	if (await exists(archivePath)) {
		await invoke("extract_file", { archive: archivePath, dest: dest });
		remove(archivePath);
	} else {
		error(`extractFile: "${archivePath}" does not exist`);
	}
};

export const getFileHash = async (path: string): Promise<string> => {
	return new Promise((resolve, reject) => {
		invoke<string>("get_sha256_sum", {
			path: path,
		})
			.then((res) => {
				resolve(res);
			})
			.catch(reject);
	});
};
