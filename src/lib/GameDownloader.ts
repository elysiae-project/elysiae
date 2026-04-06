import { join } from "@tauri-apps/api/path";
import {
	SophonProgress,
	Variants,
} from "../types";
import { getActiveGameCode, getGameExeName } from "../util/AppFunctions";
import { exists, mkdir } from "./Fs";
import { invoke } from "@tauri-apps/api/core";
import { getSettingValue } from "../util/Settings";
import { runExeWithJadeite, runExeWithWine } from "./WineManager";
import { error, info, warn } from "@tauri-apps/plugin-log";
import { listen } from "@tauri-apps/api/event";

export const downloadGame = async (game: Variants): Promise<void> => {
	const gameCode = getActiveGameCode(game);
	info("Check if dir exists");

	const gameDir = await join("games", gameCode);
	if (!(await exists(gameDir))) {
		await mkdir(gameDir);
	}

	const requestedLanguage = (await getSettingValue("voLanguage")) as string;

	const unlisten = await listen("sophon://progress", (event) => {
		const progress = event.payload as SophonProgress;
		switch (progress.type) {
			case "fetchingManifest":
				console.log("Fetching manifest...");
				break;
			case "downloading": {
				const downloaded = progress.downloaded_bytes / 1024 ** 2;
				const total = progress.total_bytes / 1024 ** 2;
				const percentage = ((downloaded / total) * 100).toFixed(2);

				info(
					`Downloaded ${downloaded.toFixed(2)}MB/${total.toFixed(2)}MB (${percentage}%)`,
				);
				break;
			}
			case "assembling": {
				const assembled = progress.assembled_files;
				const total = progress.total_files;
				info(
					`Assembling file ${assembled} of ${total} (${((assembled / total) * 100).toFixed(2)}% Complete)`,
				);
				break;
			}
			case "warning":
				warn(progress.message);
				break;
			case "error":
				error(progress.message);
				break;
			case "finished":
				info(`Download of ${gameCode} completed.`);
				unlisten(); // stop listening once complete
				break;
		}
	});
	try {
		info("Beginning sophon download sequence");
		await invoke("sophon_download", {
			gameId: gameCode,
			voLang: requestedLanguage,
			outputPath: gameDir,
		});
	} finally {
		unlisten();
	}
};

export const runGame = async (game: Variants) => {
	const gamePath = await join(
		"games",
		getActiveGameCode(game),
		getGameExeName(game),
	);
	[Variants.BH3, Variants.HKRPG].includes(game)
		? await runExeWithJadeite(gamePath)
		: await runExeWithWine(gamePath);
};

export const pauseDownload = async () => {};

export const cancelDownload = async () => {};


export const isGameInstalled = async (game: Variants): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		join("games", getActiveGameCode(game), getGameExeName(game)).then(
			(path) => {
				exists(path).then(resolve).catch(reject);
			},
		);
	});
};
