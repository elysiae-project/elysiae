import { dirname, join } from "@tauri-apps/api/path";
import { SophonChunk, SophonChunkData, Variants } from "../types";
import { getActiveGameCode, getGameExeName } from "../util/AppFunctions";
import { exists, mkdir, remove } from "./Fs";
import { invoke } from "@tauri-apps/api/core";
import { getSettingValue } from "../util/Settings";
import pLimit from "p-limit";
import { runExeWithJadeite, runExeWithWine } from "./WineManager";
import { getMd5Hash } from "../util/FileUtils";
import { info } from "@tauri-apps/plugin-log";

export const downloadGame = async (game: Variants): Promise<void> => {
	const gameCode = getActiveGameCode(game);
	info("Check if dir exists");

	const gameDir = await join("games", gameCode);
	if (!(await exists(gameDir))) {
		await mkdir(gameDir);
	}

	const requestedLanguage = (await getSettingValue("voLanguage")) as string;
	const gameChunks: SophonChunk[] = await getGameChunks(
		gameCode,
		requestedLanguage,
	);
	info("Got Game Chunks");

	let downloadedBytes = 0;
	const totalSize = getDownloadSize(gameChunks);

	const fileLimit = pLimit(8);
	const chunkLimit = pLimit(32);

	info("Starting thread mapping");
	const fileDownloadTasks = gameChunks.map((file) =>
		fileLimit(async () => {
			const path = await join("games", gameCode, file.filename);

			if (await exists(path)) {
				const localMd5 = await getMd5Hash(path);
				if (localMd5 === file.md5) {
					//info(`File ${file.filename} Downloaded. Skipping.`);
					downloadedBytes += getChunkSize(file.chunks);
					return;
				} else {
					info(`File ${file.filename} needs updating. Removing...`);
					await remove(path);
				}
			}

			const parentDir = await dirname(path);
			if (!(await exists(parentDir))) {
				await mkdir(parentDir);
			}

			const fileId = await invoke<number>("open_file", {
				path,
				totalSize: file.size,
			});

			try {
				await Promise.all(
					file.chunks.map((chunk) =>
						chunkLimit(async () => {
							await invoke("download_and_write_chunk", {
								fileId,
								cdnUrl: chunk.cdn_url,
								offset: chunk.offset,
							});
							downloadedBytes += chunk.uncompressed_size;
							info(
								`${(downloadedBytes / 1024 ** 2).toFixed(1)}MB / ${(totalSize / 1024 ** 2).toFixed(1)}MB (${((downloadedBytes / totalSize) * 100).toFixed(1)}%)`,
							);
						}),
					),
				);
			} finally {
				await invoke("close_file", { id: fileId });
			}
		}),
	);

	await Promise.all(fileDownloadTasks);
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
