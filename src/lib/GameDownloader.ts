import { dirname, join } from "@tauri-apps/api/path";
import {
	SophonChunk,
	SophonChunkData,
	SophonProgress,
	Variants,
} from "../types";
import { getActiveGameCode, getGameExeName } from "../util/AppFunctions";
import { exists, mkdir, remove } from "./Fs";
import { invoke } from "@tauri-apps/api/core";
import { getSettingValue } from "../util/Settings";
import pLimit from "p-limit";
import { runExeWithJadeite, runExeWithWine } from "./WineManager";
import { getMd5Hash } from "../util/FileUtils";
import { info } from "@tauri-apps/plugin-log";
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
			case "downloading":
				const downloaded = progress.downloadedBytes / 1024 ** 2;
				const total = progress.totalBytes / 1024 ** 2;

				console.log(
					`Downloading: ${downloaded.toFixed(2)}MB / ${total.toFixed(2)}MB (${(downloaded / total).toFixed(2)}%)`,
				);
				break;
			case "assembling":
				console.log(
					`Assembling: ${progress.assembledFiles} / ${progress.totalFiles}`,
				);
				break;
			case "warning":
				console.warn(progress.message);
				break;
			case "error":
				console.error(progress.message);
				break;
			case "finished":
				console.log("Done!");
				unlisten(); // stop listening once complete
				break;
		}

		//info("DOWNLOADING!!!!!!!!!!!");
	});
	try {
		info("Beginning sophon download sequence");
		await invoke("sophon_download", {
			gameId: gameCode,
			voLang: requestedLanguage,
			outputPath: "games/hkrpg",
		});
	} finally {
		unlisten();
	}
};

const getChunkSize = (chunkData: SophonChunkData[]) => {
	let res = 0;
	for (let i = 0; i < chunkData.length; i++) {
		res += chunkData[i].uncompressed_size;
	}
	return res;
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

export const getGameChunks = async (
	game: string,
	voLanguage: string,
): Promise<SophonChunk[]> => {
	return new Promise((resolve, reject) => {
		invoke("get_all_chunks", {
			gameId: game.toLowerCase(),
			voLang: voLanguage.toLowerCase(),
		})
			.then((res) => {
				resolve(res as SophonChunk[]);
			})
			.catch(reject);
	});
};

export const pauseDownload = async () => {};

export const cancelDownload = async () => {};

const getDownloadSize = (chunks: SophonChunk[]): number => {
	let totalSize = 0;
	chunks.forEach((chunk) => {
		totalSize += chunk.size;
	});
	return totalSize;
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
