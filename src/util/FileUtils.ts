import { invoke } from "@tauri-apps/api/core";
import { exists } from "@tauri-apps/plugin-fs";
import { error } from "@tauri-apps/plugin-log";
import { Command } from "@tauri-apps/plugin-shell";

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

export const extractFile = async (file: string, destination: string) => {
	return new Promise((resolve, reject) => {
		exists(file).then((fileExists) => {
			if (fileExists) {
				invoke("extract_file", {
					archive: file,
					destination: destination,
				})
					.then((res) => {
						resolve(res);
					})
					.catch((e) => {
						error(`File Extraction Failed!: ${e}`);
						reject(`File Extraction Error: ${e}`);
					});
			} else {
				error(`File ${file} not found`);
				reject("File Not Found");
			}
		});
	});
};

export const moveDirItems = async (
	itemsDir: string,
	newLocation: string,
	removeOriginal: boolean = true,
) => {
	// I wanted to be clever and not use shell commands but here we are.
	Command.create("sh", ["-c", `mv -v "${itemsDir}"/* "${newLocation}"`])
		.execute()
		.then(() => {
			if (removeOriginal) {
				Command.create("rm", ["-rf", itemsDir])
					.execute()
					.then(() => {
						console.log("Done");
					});
			}
		})
		.catch((e) => {
			console.error(e);
		});
};

export const getTopLevelfiles = async (location: string): Promise<string[]> => {
	return new Promise((resolve, reject) => {
		exists(location).then((locationExists) => {
			if (locationExists) {
				invoke("get_top_level_files", {
					path: location,
				}).then((res) => {
					resolve(res as string[]);
				});
			} else {
				reject(`getTopLevelFiles: Location ${location} does not exist`);
			}
		});
	});
};
