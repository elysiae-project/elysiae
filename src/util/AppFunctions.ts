import { getCurrentWindow } from "@tauri-apps/api/window";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { Command } from "@tauri-apps/plugin-shell";

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
		case Variants.BH:
			return "bh";
		case Variants.YS:
			return "ys";
		case Variants.SR:
			return "sr";
		case Variants.NAP:
			return "nap";
	}
};

export const getGameExeName = (currentGame: Variants): string => {
	switch (currentGame) {
		case Variants.BH:
			return "BH3.exe";
		case Variants.YS:
			return "GenshinImpact.exe";
		case Variants.SR:
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

export const executeShellCommand = async (
	command: string,
	env?: Record<string, string> | undefined,
) => {
	await Command.create("sh", ["-c", command], {
		env: env,
	}).execute();
};

export const convertToWinPath = (path: string) => {
	return `Z:\\${path.split("/").join("\\")}`;
};

export const convertToPosixPath = (path: string) => {
	return `/${path.substring(3, path.length).split("\\").join("/")}`;
};
