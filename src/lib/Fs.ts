import { invoke } from "@tauri-apps/api/core";
import { join } from "@tauri-apps/api/path";
import {
	BaseDirectory as BaseDir,
	type DirEntry,
	exists as tauriExists,
	mkdir as tauriMkdir,
	readDir as tauriReadDir,
	readFile as tauriReadFile,
	readTextFile as tauriReadTextFile,
	remove as tauriRemove,
	rename as tauriRename,
	writeFile as tauriWriteFile,
	writeTextFile as tauriWriteTextFile,
} from "@tauri-apps/plugin-fs";
import { error, info } from "@tauri-apps/plugin-log";
import type { Variants } from "../types";
import { variantToGameCode } from "./VariantConverter";

export const exists = async (path: string): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		tauriExists(path, { baseDir: BaseDir.AppData }).then(resolve).catch(reject);
	});
};

export const readFile = async (
	path: string,
): Promise<Uint8Array<ArrayBuffer>> => {
	return new Promise((resolve, reject) => {
		tauriReadFile(path, { baseDir: BaseDir.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const readTextFile = async (path: string): Promise<string> => {
	return new Promise((resolve, reject) => {
		tauriReadTextFile(path, { baseDir: BaseDir.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const writeFile = async (
	path: string,
	contents:
		| ReadableStream<Uint8Array<ArrayBufferLike>>
		| Uint8Array<ArrayBufferLike>,
): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriWriteFile(path, contents, {
			baseDir: BaseDir.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const writeTextFile = async (
	path: string,
	contents: string,
): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriWriteTextFile(path, contents, {
			baseDir: BaseDir.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const remove = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, { baseDir: BaseDir.AppData }).then(resolve).catch(reject);
	});
};

export const removeDir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, { recursive: true, baseDir: BaseDir.AppData })
			.then(resolve)
			.catch(reject);
	});
};

export const mkdir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriMkdir(path, { baseDir: BaseDir.AppData, recursive: true })
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
			newPathBaseDir: BaseDir.AppData,
			oldPathBaseDir: BaseDir.AppData,
		})
			.then(resolve)
			.catch(reject);
	});
};

export const readDir = async (path: string): Promise<DirEntry[]> => {
	return new Promise((resolve, reject) => {
		tauriReadDir(path, { baseDir: BaseDir.AppData })
			.then((res) => {
				resolve(res);
			})
			.catch(reject);
	});
};

export const getDirFileNames = async (path: string): Promise<string[]> => {
	return new Promise((resolve, reject) => {
		readDir(path)
			.then((dirItems) => {
				const final: string[] = [];
				dirItems.map((i) => final.push(i.name));
				resolve(final);
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

export const getDirSize = async (game: Variants): Promise<number> => {
	return new Promise((resolve, reject) => {
		join("games", variantToGameCode[game])
			.then((gameDir) => {
				invoke("get_dir_size", {
					path: gameDir,
				})
					.then((res) => {
						resolve((res as number) / 1024 ** 3);
					})
					.catch(reject);
			})
			.catch(reject);
	});
};
