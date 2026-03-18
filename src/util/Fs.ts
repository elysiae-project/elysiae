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

export const exists = async (path: string): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		tauriExists(path)
			.then((res) => {
				resolve(res as boolean);
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const writeFile = async (
	path: string,
	contents:
		| Uint8Array<ArrayBufferLike>
		| ReadableStream<Uint8Array<ArrayBufferLike>>,
): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriWriteFile(path, contents, {
			baseDir: BaseDirectory.AppData,
		})
			.then(() => {
				resolve();
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const writeTextFile = async (
	path: string,
	contents: string,
): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriWriteTextFile(path, contents, {
			baseDir: BaseDirectory.AppData,
		})
			.then(() => {
				resolve();
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const readFile = async (
	path: string,
): Promise<Uint8Array<ArrayBuffer>> => {
	return new Promise((resolve, reject) => {
		tauriReadFile(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then((res) => {
				resolve(res as Uint8Array<ArrayBuffer>);
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const readTextFile = async (path: string): Promise<string> => {
	return new Promise((resolve, reject) => {
		tauriReadTextFile(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then((res) => {
				resolve(res as string);
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const remove = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, {
			baseDir: BaseDirectory.AppData
		})
			.then(() => {
				resolve();
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const removeDir = async(path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRemove(path, {
			recursive: true,
			baseDir: BaseDirectory.AppData
		}).then(() => {
			resolve();
		}).catch((e) => {
			reject(e);
		})
	})
}

export const mkdir = async (path: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriMkdir(path, {
			baseDir: BaseDirectory.AppData,
		})
			.then(() => {
				resolve();
			})
			.catch((e) => {
				reject(e);
			});
	});
};

export const rename = async (originalPath: string, destPath: string): Promise<void> => {
	return new Promise((resolve, reject) => {
		tauriRename(originalPath, destPath, {
			newPathBaseDir: BaseDirectory.AppData,
			oldPathBaseDir: BaseDirectory.AppData,
		}).then(() => {
			resolve();
		}).catch((e) => {
			reject(e);
		});
	});
};
