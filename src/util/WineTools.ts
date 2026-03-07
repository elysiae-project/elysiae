import { getApiJson, getGithubInfo } from "./WebUtils";
import { singleDownload } from "./DownloadManager";
import {
	exists,
	mkdir,
	readTextFile,
	remove,
	rename,
	writeTextFile,
} from "@tauri-apps/plugin-fs";
import { basename, join, resourceDir } from "@tauri-apps/api/path";
import { extractFile, getAllDirs, moveDirItems } from "./FileUtils";
import { Command } from "@tauri-apps/plugin-shell";
import { error, info, warn } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";

type WineAsset = "wine" | "vkd3d" | "jadeite";
/**
 * @description Creates a wine environment
 */
export const createWineEnv = async (): Promise<void> => {
	// While not necessarily part of the wine environment,
	// Jadeite should be downloaded alongside it
	const commands = [
		updateJadeite,
		updateWine,
		updateWinetricks,
		updateWinetricksModules,
		updateVkd3d,
	] as const;
	for (let i = 0; i < commands.length; i++) {
		info(`Running ${commands[i].name}`);
		await commands[i]().catch((e) => {
			error(`createWineEnv: ${e}`);
			return;
		});
	}
	info("Wine Environment Creation Completed");
};

export const updateJadeite = async (): Promise<void> => {
	const appDir = await resourceDir();
	const downloadLocation = await join(appDir, "jadeite.zip");
	const extractLocation = await join(appDir, "jadeite");

	const repoInfo = await getApiJson(
		"https://codeberg.org/api/v1/repos/mkrsym1/jadeite/releases/latest",
	);
	const url: string = repoInfo.assets[0].browser_download_url;
	const tagName = repoInfo.tag_name;
	const releaseHash = repoInfo.assets[0].uuid; // I guess this works good enough as a hash replacement.

	await singleDownload(url, downloadLocation);
	await extractFile(downloadLocation, extractLocation);

	await updateAssetTracker("jadeite", {
		tag: tagName,
		hash: releaseHash,
	});

	// The second half of this requires admin elevation to setup and must be performed
	const script = await join(extractLocation, "block_analytics.sh");
	const result = await Command.create("sh", [script]).execute();

	if (result.code !== 0) {
		await remove(extractLocation);
		await remove(downloadLocation);
		throw new Error("User Admin Prompt Failed");
	}

	await remove(downloadLocation);
};

/**
 * Updates the current wine install
 */
export const updateWine = async (): Promise<void> => {
	const appDir = await resourceDir();
	const downloadLocation = await join(appDir, "wine.tar.xz");
	const extractLocation = await join(appDir, "wine-temp");
	const finalLocation = await join(appDir, "wine");

	if (!(await exists(finalLocation))) {
		await mkdir(finalLocation);
	}

	const repoInfo = await getGithubInfo(
		"https://api.github.com/repos/NelloKudo/spritz-wine-aur/releases/latest",
	);

	await singleDownload(repoInfo.downloadURL, downloadLocation);
	await extractFile(downloadLocation, extractLocation);

	const folder = (await getAllDirs(extractLocation))[0];
	await moveDirItems(folder, finalLocation);

	// Quickly (re)-generate a wineprefix. Useful even when updating
	await Command.create("sh", ["-c", `${finalLocation}/bin/wineboot -i`], {
		env: {
			WINEPREFIX: finalLocation,
		},
	}).execute();

	await Command.create("sh", ["-c", `${finalLocation}/bin/wineserver --wait`], {
		env: {
			WINEPREFIX: finalLocation,
		},
	}).execute();

	await remove(downloadLocation);
	await remove(extractLocation, {
		recursive: true,
	});

	await updateAssetTracker("wine", {
		tag: repoInfo.tagName,
		hash: repoInfo.hash,
	});
};

/**
 * @description Downloads winetricks
 */
export const updateWinetricks = async (): Promise<void> => {
	const appDir = await resourceDir();
	const wineDir = await join(appDir, "wine");
	const downloadLocation = await join(wineDir, "winetricks");
	if (!(await exists(wineDir))) {
		await updateWine();
	}

	await singleDownload(
		"https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks",
		downloadLocation,
	);
};

/**
 * @description Sets up vkd3d in the wine environment. if no wine environment exists, create it first.
 */
export const updateVkd3d = async (): Promise<void> => {
	const appDir = await resourceDir();
	const downloadLocation = await join(appDir, "wine.tar.zst");
	const extractLocation = await join(appDir, "vkd3d-proton-temp");
	const wineDir = await join(appDir, "wine");

	const repoInfo = await getGithubInfo(
		"https://api.github.com/repos/HansKristian-Work/vkd3d-proton/releases/latest",
	);

	await singleDownload(repoInfo.downloadURL, downloadLocation);
	await extractFile(downloadLocation, extractLocation);
	const folder = (await getAllDirs(extractLocation))[0];
	await moveDirItems(folder, extractLocation);

	if (!(await exists(wineDir))) {
		await createWineEnv();
		return;
	}

	const dirs = [
		{
			initialLocation: await join(extractLocation, "x64"),
			moveTo: await join(wineDir, "drive_c", "windows", "system32"),
		},
		{
			initialLocation: await join(extractLocation, "x86"),
			moveTo: await join(wineDir, "drive_c", "windows", "syswow64"),
		},
	] as const;

	for (let i = 0; i < dirs.length; i++) {
		const files = await invoke<string[]>("get_all_files", {
			path: dirs[i].initialLocation,
		});

		for (let j = 0; j < files.length; j++) {
			const fileName = await basename(files[j]);
			const finalLocation = await join(dirs[i].moveTo, fileName);
			await rename(files[j], finalLocation);
		}
	}
	// Adds required registry keys for vkd3d12 to work
	// d3d12
	await wineCommand(
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12 /t REG_SZ /d native /f`,
	);

	// d3d12core
	await wineCommand(
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12core /t REG_SZ /d native /f`,
	);

	await remove(downloadLocation);
	await remove(extractLocation, {
		recursive: true,
	});

	await updateAssetTracker("vkd3d", {
		tag: repoInfo.tagName,
		hash: repoInfo.hash,
	});
};

/**
 * Installs/Updates winetricks modules used in yoohoo
 */
export const updateWinetricksModules = async (): Promise<void> => {
	// Trying to install a few different redists to ensure that older games will still run properly.
	// Not sure if just vcrun2022/vcrun2026 will be able to do this
	await winetricksCommand("vcrun2022 vcrun2026 dxvk mfc140");
};

/**
 * executes a command with __wine__
 * @param commands list of commands to run with ``wine``
 */
export const wineCommand = async (commands: string): Promise<void> => {
	// These operators are typically used to chain shell commands together
	if (isCommandValid(commands)) {
		warn(
			`The command ${commands} includes one or more of: &&, &, ;. This is not allowed`,
		);
		return;
	}
	const appDir = await resourceDir();
	const winePrefix = await join(appDir, "wine");
	if (!(await wineEnvAvailable())) {
		throw new Error("Wine Does not exist");
	}

	await Command.create("sh", ["-c", `${winePrefix}/bin/wine ${commands}`], {
		env: {
			WINEFSYNC: "1",
			WINEPREFIX: winePrefix,
		},
	})
		.execute()
		.catch((e) => {
			error(`wineCommand: ${e}`);
		});
};

/**
 * Executes a command with __winetricks__
 * @param commands list of commands to run
 */
export const winetricksCommand = async (commands: string): Promise<void> => {
	if (isCommandValid(commands)) {
		warn(
			`The command ${commands} includes one or more of: &&, &, ;. This is not allowed`,
		);
		return;
	}
	const appDir = await resourceDir();

	const winePrefix = await join(appDir, "wine");
	if (!(await wineEnvAvailable())) {
		throw new Error("Wine env does not exist");
	}

	await Command.create(
		"sh",
		["-c", `${winePrefix}/winetricks -q ${commands}`],
		{
			env: {
				WINEFSYNC: "1",
				WINEPREFIX: winePrefix,
			},
		},
	).execute();
};

/**
 * Checks if a command does not contain any keywords that may trigger the execution of unintended shell commands
 * @param command any shell command ``string``
 * @returns value based on if the command does not inclue ``&&``, ``&``, or ``;``
 */
const isCommandValid = (command: string) => {
	return (
		command.includes("&&") || command.includes("&") || command.includes(";")
	);
};

/**
 * @returns value based on if the wine prefix directory exists
 */
export const wineEnvAvailable = async (): Promise<boolean> => {
	const wineBinary = await join(await resourceDir(), "wine", "bin", "wine");
	if (await exists(wineBinary)) {
		const testResult = await Command.create("sh", ["-c", wineBinary]).execute();
		return testResult.code == 1;
	}
	return false;
};

/**
 * @description Creates/Updates wine asset tracker used for component updates
 * @param tag Which wine asset entry
 * @param info Object for release tag/version (``tag``) and the sha256sum of the current download package (``hash``)
 */
export const updateAssetTracker = async (
	tag: WineAsset,
	info: {
		tag: string;
		hash: string;
	},
): Promise<void> => {
	const appDir = await resourceDir();
	const assetFile = await join(appDir, "assets.json");

	if (!(await exists(assetFile))) {
		await writeTextFile(assetFile, "{}"); // Create an "Empty" asset file
	}

	const fileData = await readTextFile(assetFile);
	const json = JSON.parse(fileData);
	json[tag] = info;

	await writeTextFile(assetFile, JSON.stringify(json));
};

export const convertToWinPath = (path: string) => {
	return `Z:\\${path.split("/").join("\\")}`;
};

export const convertToPosixPath = (path: string) => {
	return `/${path.substring(3, path.length).split("\\").join("/")}`;
};
