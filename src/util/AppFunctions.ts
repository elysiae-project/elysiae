import { getCurrentWindow } from "@tauri-apps/api/window";
import { Variants } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { Command } from "@tauri-apps/plugin-shell";
import { appDataDir } from "@tauri-apps/api/path";

/**
 * @description Closes the app
 */
export const closeApp = (): void => {
	getCurrentWindow().close();
};

/**
 * @description Minimizes the app window
 */
export const minimizeApp = (): void => {
	getCurrentWindow().minimize();
};

/**
 * @returns Game codes (in type ``Variants``) as string (``bh/ys/sr/nap``)
 */
export const getActiveGameCode = (
	currentGame: Variants,
): "bh" | "ys" | "sr" | "nap" => {
	switch (currentGame) {
		case Variants.BH3:
			return "bh";
		case Variants.HK4E:
			return "ys";
		case Variants.HKRPG:
			return "sr";
		case Variants.NAP:
			return "nap";
	}
};

export const getGameExeName = (currentGame: Variants): string => {
	switch (currentGame) {
		case Variants.BH3:
			return "BH3.exe";
		case Variants.HK4E:
			return "GenshinImpact.exe";
		case Variants.HKRPG:
			return "StarRail.exe";
		case Variants.NAP:
			return "ZenlessZoneZero.exe";
	}
};

/**
 * @returns ``boolean`` value based on weather or not the app is running in a development environment
 */
export const inDevEnv = async (): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		invoke("in_dev_env")
			.then((res) => {
				resolve(res as boolean);
			})
			.catch((e) => {
				reject(e);
			});
	});
};

/**
 * Executes a command on the shell
 * @param command any command
 * @param env (optional) environment variables
 */
export const executeShellCommand = async (
	command: string,
	env?: Record<string, string> | undefined,
) => {
	await Command.create("sh", ["-c", command], {
		env: env,
	}).execute();
};

/**
 * Executes a command of a binary found in the Elysiae's app data directory
 * @param binaryPath path to binary, relative to the app data directory
 * @param args arguments to pass into command
 * @param env (optional) environment variables
 */
export const executeLocalCommand = async (
	binaryPath: string,
	args?: string,
	env?: Record<string, string> | undefined,
) => {
	const appData = await appDataDir();
	await executeShellCommand(
		`${appData}/${binaryPath} ${typeof args !== "undefined" ? args : ""}`,
		env,
	).catch((e) => {
		throw new Error(e);
	});
};

/**
 * Convert a POSIX path to a Windows path used by Wine
 * @param path POSIX Path
 * @returns Wine Windows path converted froma POSIX path
 */
export const convertToWinPath = (path: string) => {
	return `Z:\\${path.split("/").join("\\")}`;
};

/**
 * Convert a Windows path used by Wine to POSIX
 * @param path Wine Windows Path
 * @returns POSIX path converted from a Wine Windows Path
 */
export const convertToPosixPath = (path: string) => {
	return `/${path.substring(3, path.length).split("\\").join("/")}`;
};
