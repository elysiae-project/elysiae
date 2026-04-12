import { invoke } from "@tauri-apps/api/core";
import { BaseDirectory } from "@tauri-apps/plugin-fs";
import {
	exists as tauriExists,
	writeTextFile as tauriWriteTextFile,
	writeFile as tauriWriteFile,
	readFile as tauriReadFile,
	readTextFile as tauriReadTextFile,
	remove as tauriRemove,
	mkdir as tauriMkdir,
	rename as tauriRename,
} from "@tauri-apps/plugin-fs";
import { error, info } from "@tauri-apps/plugin-log";

/**
 * Checks if a folder exists, relative to the app data directory
 * @param path path to file/folder
 * @returns weather or not a path  exists
 */
export const exists = async (path: string): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		tauriExists(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

/**
 * Write to a file within the app data directory
 * @param path path to write to (relative to app data dir)
 * @param contents Contents to write to the file, Can be binary or text
 */
export const writeFile = async (
	path: string,
	contents:
		| Uint8Array<ArrayBufferLike>
		| ReadableStream<Uint8Array<ArrayBufferLike>>
		| string,
	appendContents: boolean = false,
) => {
	const writeFunction =
		typeof contents === "string" ? tauriWriteTextFile : tauriWriteFile;
	await writeFunction(path, contents as any, {
		baseDir: BaseDirectory.AppData,
		append: appendContents,
	});
};

export const readFile = async (
	path: string,
): Promise<Uint8Array<ArrayBuffer>> => {
	return new Promise((resolve, reject) => {
		tauriReadFile(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const readTextFile = async (path: string): Promise<string> => {
	return new Promise((resolve, reject) => {
		tauriReadTextFile(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const remove = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const removeDir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, {
			recursive: true,
			baseDir: BaseDirectory.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const mkdir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriMkdir(path, {
			baseDir: BaseDirectory.AppData,
			recursive: true,
		})
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
		error(`extractFile: "${archivePath}" does not exist`);
	}
};
