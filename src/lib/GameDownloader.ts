import { join } from "@tauri-apps/api/path";
import { GameData, ResumeInfo, SophonProgress, Variants } from "../types";
import {
	getActiveGameCode,
	getGameExeName,
	getGameName,
} from "../util/AppFunctions";
import { exists } from "./Fs";
import { invoke } from "@tauri-apps/api/core";
import { getOption } from "../util/Settings";
import { runExeWithJadeite, runExeWithWine } from "./WineManager";
import { error, info, warn } from "@tauri-apps/plugin-log";
import { listen } from "@tauri-apps/api/event";
import { broadcastNotification } from "../util/NotificationHelper";

/**
 * Downloads a fresh install of any game to `games/gameCode`
 * @param game
 */
export const downloadGame = async (game: Variants): Promise<void> => {
	const gameData = await getGameData(game);
	await broadcastNotification(`Beginning Download of ${getGameName(game)}`);
const unlisten = await listen("sophon://progress", (event) => {
    const progress = event.payload as SophonProgress;
    switch (progress.type) {
      case "fetchingManifest":
        info("Fetching manifest...");
        break;
      case "downloading": {
        const downloaded = progress.downloaded_bytes / 1024 ** 2;
        const total = progress.total_bytes / 1024 ** 2;
        const percentage = ((downloaded / total) * 100).toFixed(2);
        const speed = progress.speed_bps / 1024 ** 2;
        const eta = progress.eta_seconds;

        const etaStr = eta > 0
          ? eta >= 3600
            ? `${Math.floor(eta / 3600)}h ${Math.floor((eta % 3600) / 60)}m`
            : `${Math.floor(eta / 60)}m ${Math.floor(eta % 60)}s`
          : "calculating...";

        info(
          `Downloaded ${downloaded.toFixed(2)}MB/${total.toFixed(2)}MB (${percentage}%) - ${speed.toFixed(2)}MB/s - ETA: ${etaStr}`,
        );
        break;
      }
      case "paused": {
        const downloaded = progress.downloaded_bytes / 1024 ** 2;
        const total = progress.total_bytes / 1024 ** 2;
        info(`Download paused at ${downloaded.toFixed(2)}MB/${total.toFixed(2)}MB`);
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
      case "verifying": {
        const scanned = progress.scanned_files;
        const total = progress.total_files;
        const errors = progress.error_count;
        info(
          `Verifying files: ${scanned}/${total} scanned, ${errors} errors found`,
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
		await broadcastNotification(
			`${getGameName(game)} Has Finished Downloading`,
		);
	}
};

/**
 * Launches a game with wine. Games that require Jadeite are launched with jadeite instead
 * @param game
 */
export const runGame = async (game: Variants): Promise<void> => {
	const gamePath = await join(
		"games",
		getActiveGameCode(game),
		getGameExeName(game),
	);
	[Variants.BH3, Variants.HKRPG].includes(game)
		? await runExeWithJadeite(gamePath)
		: await runExeWithWine(gamePath);
};

/**
 * Pause Sophon Chunk Download
 */
export const pauseDownload = async (): Promise<void> => {
	await invoke("sophon_pause");
};

/**
 * Resume Sophon Chunk Download
 */
export const resumeDownload = async (): Promise<void> => {
	await invoke("sophon_resume");
};

/**
 * Cancel Sophon Chunk Download (All downloaded chunks will be deleted)
 */
export const cancelDownload = async (): Promise<void> => {
	await invoke("sophon_cancel");
};

/**
 * Check if a preinstall for a specified game is available
 * @param game The game in question
 * @returns weather or not a preinstall for the specified game is available
 */
export const isPreinstallAvailable = async (
  game: Variants,
): Promise<boolean> => {
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

/**
 * Check if a game update is available
 * @param game The game to check
 * @returns Object containing update availability info, or null if not installed
 */
export const checkGameUpdate = async (
  game: Variants,
): Promise<{
  updateAvailable: boolean;
  currentVersion: string | null;
  newVersion: string;
  updateSize: number;
  preinstallAvailable: boolean;
  preinstallDownloaded: boolean;
} | null> => {
  const gameData = await getGameData(game);

  try {
    const infoData = await invoke<{
      update_available: boolean;
      current_tag: string | null;
      remote_tag: string;
      update_compressed_size: number;
      preinstall_available: boolean;
      preinstall_downloaded: boolean;
    }>("sophon_check_update", {
      gameId: gameData.gameCode,
      voLang: gameData.requestedLanguage,
      outputPath: gameData.gameDir,
    });

    return {
      updateAvailable: infoData.update_available,
      currentVersion: infoData.current_tag,
      newVersion: infoData.remote_tag,
      updateSize: infoData.update_compressed_size,
      preinstallAvailable: infoData.preinstall_available,
      preinstallDownloaded: infoData.preinstall_downloaded,
    };
  } catch {
    // Game not installed
    return null;
  }
};

/**
 * Check if there is a download state to resume (from app crash/close)
 */
export const hasResumeState = async (): Promise<boolean> => {
  return invoke<boolean>("sophon_has_resume_state");
};

export const getResumeInfo = async (): Promise<ResumeInfo | null> => {
  return invoke<ResumeInfo | null>("sophon_get_resume_info");
};

export const resumeDownloadInterrupted = async (): Promise<void> => {
  await broadcastNotification("Resuming interrupted download...");
  const unlisten = await listen("sophon://progress", (event) => {
    const progress = event.payload as SophonProgress;
    switch (progress.type) {
      case "fetchingManifest":
        info("Fetching manifest for resume...");
        break;
      case "downloading": {
        const downloaded = progress.downloaded_bytes / 1024 ** 2;
        const total = progress.total_bytes / 1024 ** 2;
        const percentage = ((downloaded / total) * 100).toFixed(2);
        const speed = progress.speed_bps / 1024 ** 2;
        const eta = progress.eta_seconds;
        const etaStr = eta > 0
          ? eta >= 3600
            ? `${Math.floor(eta / 3600)}h ${Math.floor((eta % 3600) / 60)}m`
            : `${Math.floor(eta / 60)}m ${Math.floor(eta % 60)}s`
          : "calculating...";
        info(
          `Resuming: ${downloaded.toFixed(2)}MB/${total.toFixed(2)}MB (${percentage}%) - ${speed.toFixed(2)}MB/s - ETA: ${etaStr}`,
        );
        break;
      }
      case "paused": {
        const downloaded = progress.downloaded_bytes / 1024 ** 2;
        const total = progress.total_bytes / 1024 ** 2;
        info(`Download paused at ${downloaded.toFixed(2)}MB/${total.toFixed(2)}MB`);
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
      case "verifying": {
        const scanned = progress.scanned_files;
        const total = progress.total_files;
        const errors = progress.error_count;
        info(
          `Verifying files: ${scanned}/${total} scanned, ${errors} errors found`,
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
        info("Download completed.");
        unlisten();
        break;
    }
  });
  try {
    await invoke("sophon_resume_download");
  } finally {
    unlisten();
    await broadcastNotification("Resume complete");
  }
};

/**
 * Downloads an update/preinstall for a specified game
 * @param game The specified game
 * @param isPreinstall weather or not the download is for a preinstall
 */
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

/**
 * Applies a preinstall for a game, if available
 * @param game Game to apply preinstall to
 */
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

/**
 * Verify game file integrity and re-download corrupted files
 * @param game The game to verify
 */
export const verifyGameIntegrity = async (game: Variants): Promise<void> => {
  const gameData = await getGameData(game);
  await broadcastNotification(`Verifying ${getGameName(game)} integrity...`);

  const unlisten = await listen("sophon://progress", (event) => {
    const progress = event.payload as SophonProgress;
    switch (progress.type) {
      case "verifying": {
        const scanned = progress.scanned_files;
        const total = progress.total_files;
        const errors = progress.error_count;
        info(`Verifying: ${scanned}/${total} files scanned, ${errors} errors found`);
        break;
      }
      case "warning":
        warn(progress.message);
        break;
      case "error":
        error(progress.message);
        break;
      case "finished":
        info(`Integrity verification of ${gameData.gameCode} completed.`);
        unlisten();
        break;
    }
  });

  try {
    await invoke("sophon_verify_integrity", {
      gameId: gameData.gameCode,
      voLang: gameData.requestedLanguage,
      outputPath: gameData.gameDir,
    });
  } finally {
    unlisten();
    await broadcastNotification(`${getGameName(game)} integrity check complete`);
  }
};

/**
 * Check if a game is installed
 * @param game game to check if installed
 * @returns weather or not `game` is installed
 */
export const isGameInstalled = async (game: Variants): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		// TODO: Replace with a more effective and robust way to check for game installs
		// pkg_version is one of the last files that is downloaded
		join("games", getActiveGameCode(game), "pkg_version").then((path) => {
			exists(path).then(resolve).catch(reject);
		});
	});
};

/**
 * Returns a simple object with basic game information used by other functions in this file
 * @param game game code that is currently being used
 * @returns Information about the game code, the install directory, and what voice over language the user requested in their settings
 */
const getGameData = async (game: Variants): Promise<GameData> => {
	const gameCode = getActiveGameCode(game);
	const gameDir = await join("games", gameCode);
	const requestedLanguage = (await getOption("voLanguage")) as string;

	return {
		gameCode,
		gameDir,
		requestedLanguage,
	};
};
