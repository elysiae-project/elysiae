import { getCurrentWindow } from "@tauri-apps/api/window";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { invoke } from "@tauri-apps/api/core";

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
export const getActiveGameCode = (): "bh" | "ys" | "sr" | "nap" => {
	const { game, setGame } = useGame();

	switch (game) {
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
