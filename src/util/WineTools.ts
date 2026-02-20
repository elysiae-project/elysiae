import { downloadFile, getGithubInfo } from "./WebUtils";
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
import { info, warn } from "@tauri-apps/plugin-log";
import { invoke } from "@tauri-apps/api/core";

export const createWineEnv = async (): Promise<void> => {
	await updateWine();
	await updateWinetricks();
	await updateWinetricksModules();
	await updateVkd3d();
	info("Wine Env Creation Complete");
};

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

	await downloadFile(repoInfo.downloadURL, downloadLocation);
	await extractFile(downloadLocation, extractLocation);

	const folder = (await getAllDirs(extractLocation))[0];
	await moveDirItems(folder, finalLocation);

	// Quickly generate a wineprefix
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

export const updateWinetricks = async (): Promise<void> => {
	const appDir = await resourceDir();
	const wineDir = await join(appDir, "wine");
	const downloadLocation = await join(wineDir, "winetricks");
	if (!(await exists(wineDir))) {
		await updateWine();
	}

	await downloadFile(
		"https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks",
		downloadLocation,
	);
};

export const updateVkd3d = async (): Promise<void> => {
	// d3d12
	await wineCommand(
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12 /t REG_SZ /d native /f`,
	);

	// d3d12core
	await wineCommand(
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12core /t REG_SZ /d native /f`,
	);

	const appDir = await resourceDir();
	const downloadLocation = await join(appDir, "wine.tar.zst");
	const extractLocation = await join(appDir, "vkd3d-proton-temp");
	const wineDir = await join(appDir, "wine");

	const repoInfo = await getGithubInfo(
		"https://api.github.com/repos/HansKristian-Work/vkd3d-proton/releases/latest",
	);

	await downloadFile(repoInfo.downloadURL, downloadLocation);
	await extractFile(downloadLocation, extractLocation);
	const folder = (await getAllDirs(extractLocation))[0];
	await moveDirItems(folder, extractLocation);

	if (!(await exists(wineDir))) {
		await createWineEnv();
		return;
	}

	// Not using rename() because this is a lot cleaner
	const dirs = [
		{
			initialLocation: await join(extractLocation, "x64"),
			moveTo: await join(wineDir, "drive_c", "windows", "system32"),
		},
		{
			initialLocation: await join(extractLocation, "x86"),
			moveTo: await join(wineDir, "drive_c", "windows", "syswow64"),
		},
	];

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

	await remove(downloadLocation);
	await remove(extractLocation, {
		recursive: true,
	});

	await updateAssetTracker("vkd3d", {
		tag: repoInfo.tagName,
		hash: repoInfo.hash,
	});
};

export const updateWinetricksModules = async (): Promise<void> => {
	// Trying to install a few different redists to ensure that older games will still run properly.
	// Not sure if just vcrun2022/vcrun2026 will be able to do this
	await winetricksCommand("vcrun2019 vcrun2022 vcrun2026 dxvk");
};

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
	if (!(await exists(winePrefix))) {
		await createWineEnv();
	}

	await Command.create("sh", ["-c", `${winePrefix}/bin/wine ${commands}`], {
		env: {
			WINEPREFIX: winePrefix,
		},
	}).execute();
};

export const winetricksCommand = async (commands: string): Promise<void> => {
	if (isCommandValid(commands)) {
		warn(
			`The command ${commands} includes one or more of: &&, &, ;. This is not allowed`,
		);
		return;
	}
	const appDir = await resourceDir();

	const winePrefix = await join(appDir, "wine");
	if (!(await exists(winePrefix))) {
		await updateWine();
		await updateWinetricks();
	}

	await Command.create(
		"sh",
		["-c", `${winePrefix}/winetricks -q ${commands}`],
		{
			env: {
				WINEPREFIX: winePrefix,
			},
		},
	).execute();
};

const isCommandValid = (command: string) => {
	// These operators are typically used to chain shell commands together
	return (
		!command.includes("&&") || !command.includes("&") || !command.includes(";")
	);
};

export const wineEnvActive = async (): Promise<boolean> => {
	return new Promise((resolve) => {
		resourceDir().then((appDir) => {
			join(appDir, "wine").then((wineDir) => {
				exists(wineDir).then((res) => {
					resolve(res as boolean);
				});
			});
		});
	});
};

type wineAsset = "wine" | "vkd3d";
export const updateAssetTracker = async (
	tag: wineAsset,
	info: any,
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
