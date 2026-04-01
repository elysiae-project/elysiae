import { join } from "@tauri-apps/api/path";
import { Variants } from "../types";
import { getActiveGameCode, getGameExeName } from "../util/AppFunctions";
import { exists } from "./Fs";

export const downloadGame = async () => {};

export const runGame = async () => {};

export const isGameInstalled = async (game: Variants): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		join("games", getActiveGameCode(game), getGameExeName(game)).then(
			(path) => {
				exists(path)
					.then((res) => resolve(res as boolean))
					.catch(reject);
			},
		);
	});
};
