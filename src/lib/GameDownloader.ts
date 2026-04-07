import { join } from "@tauri-apps/api/path";
import { SophonProgress, Variants } from "../types";
import { getActiveGameCode, getGameExeName } from "../util/AppFunctions";
import { exists, mkdir } from "./Fs";
import { invoke } from "@tauri-apps/api/core";
import { getSettingValue } from "../util/Settings";
import { runExeWithJadeite, runExeWithWine } from "./WineManager";
import { error, info, warn } from "@tauri-apps/plugin-log";
import { listen } from "@tauri-apps/api/event";

type GameData = {
	gameCode: string;
	gameDir: string;
	requestedLanguage: string;
};

export const downloadGame = async (game: Variants): Promise<void> => {
	const gameData = await getGameData(game);

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
				info(`Download of ${gameData.gameCode} completed.`);
				unlisten(); // stop listening once complete
				break;
		}
	});
	try {
		info("Beginning sophon download sequence");
		await invoke("sophon_download", {
			gameId: gameData.gameCode,
			voLang: gameData.requestedLanguage,
			outputPath: gameData.gameDir,
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

export const pauseDownload = async () => {
	await invoke("sophon_pause");
};

export const resumeDownload = async () => {
	await invoke("sophon_resume");
};

export const cancelDownload = async () => {
	await invoke("sophon_cancel");
};

export const isPreinstallAvailable = async (game: Variants) => {
	const gameData = await getGameData(game);
	const infoData = await invoke<{
		preinstall_available: boolean;
		preinstall_downloaded: boolean;
	}>("sophon_check_update", {
		gameId: gameData.gameCode,
		voLang: gameData.requestedLanguage,
		outputPath: gameData.gameDir,
	});

	return infoData.preinstall_available && !infoData.preinstall_downloaded;
};

export const downloadUpdate = async (
	game: Variants,
	isPreinstall: boolean = false,
): Promise<void> => {
	const gameData = await getGameData(game);

	if (isPreinstall) {
		await invoke("sophon_preinstall", {
			gameId: gameData.gameCode,
			voLang: gameData.requestedLanguage,
			outputPath: gameData.gameDir,
		});
	} else {
		await invoke("sophon_update", {
			gameId: gameData.gameCode,
			voLang: gameData.requestedLanguage,
			outputPath: gameData.gameDir,
		});
	}
};

export const applyUpdate = async (game: Variants): Promise<void> => {
	const gameData = await getGameData(game);

	const { preinstall_tag } = await invoke<{ preinstall_tag: string | null }>(
		"sophon_check_update",
		{
			gameId: gameData.gameCode,
			voLang: gameData.requestedLanguage,
			outputPath: gameData.gameDir,
		},
	);

	if (!preinstall_tag) {
		throw new Error(`No Preinstall found for ${gameData.gameCode}`);
	}

	await invoke("sophon_apply_preinstall", {
		preinstallTag: preinstall_tag,
		outputPath: gameData.gameDir,
	});
};

export const isGameInstalled = async (game: Variants): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		join("games", getActiveGameCode(game), getGameExeName(game)).then(
			(path) => {
				exists(path).then(resolve).catch(reject);
			},
		);
	});
};

const getGameData = async (game: Variants): Promise<GameData> => {
	const gameCode = getActiveGameCode(game);
	const gameDir = await join("games", gameCode);
	const requestedLanguage = (await getSettingValue("voLanguage")) as string;

	return {
		gameCode,
		gameDir,
		requestedLanguage,
	};
};
