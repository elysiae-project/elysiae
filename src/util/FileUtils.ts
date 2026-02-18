import { invoke } from "@tauri-apps/api/core";

export const compareHash = (file: string, hashFile: string) => {};

export const extractFile = async (archive: string, destination: string) => {
	await invoke("extract_file", {
		archive: archive,
		destination: destination,
	});
};

export const isFileValid = async (file: string): Promise<boolean> => {
	return true;
};

