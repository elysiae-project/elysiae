import { downloadFile, getGithubInfo } from "./WebUtils";
import { exists, mkdir, remove } from "@tauri-apps/plugin-fs";
import { join, resourceDir } from "@tauri-apps/api/path";
import { extractFile, getAllDirs, moveDirItems } from "./FileUtils";
import { Command } from "@tauri-apps/plugin-shell";

export const createWineEnvironment = async () => {
	// Wine prefix creation should be in this order:
	// 1. Download and extract wine
	// 2. Download winetricks
	// 3. Use winetricks to install C++ redist and dxvk, which also creates a wineprefix automatically
	// 4. Install vkd3d-proton for futureproofing against future games that will use dx12.
	// 	  This install process relies on the wineprefix created in step 3
	await updateWine();
	await updateWinetricks();
	await updateWinetricksModules();
	await updateVkd3d();
};

export const updateWine = async () => {
	const appDir = await resourceDir();
	const downloadLocation = await join(appDir, "wine.tar.xz");
	const extractLocation = await join(appDir, "wine-temp");
	const finalLocation = await join(appDir, "wine");
	await mkdir(finalLocation);

	const repoInfo = await getGithubInfo(
		"https://api.github.com/repos/NelloKudo/spritz-wine-aur/releases/latest",
	);

	await downloadFile(repoInfo.downloadURL, downloadLocation);
	await extractFile(downloadLocation, extractLocation);

	const folder = (await getAllDirs(extractLocation))[0];
	await moveDirItems(folder, finalLocation);

	await remove(downloadLocation);
	await remove(extractLocation, {
		recursive: true,
	});
};

export const updateWinetricks = async () => {
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

export const updateVkd3d = async () => {
	const appDir = await resourceDir();
	const downloadLocation = await join(appDir, "wine.tar.zst");
	const extractLocation = await join(appDir, "vkd3d-proton-temp");
	const wineDir = await join(appDir, "wine");

	const repoInfo = await getGithubInfo(
		"https://api.github.com/repos/HansKristian-Work/vkd3d-proton/releases/latest",
	);

	await downloadFile(repoInfo.downloadURL, downloadLocation);
	await extractFile(downloadLocation, extractLocation);

	if (!(await exists(wineDir))) {
		await createWineEnvironment();
		return;
	}

	// Not using rename() because this is a lot cleaner
	await Command.create("sh", [
		"-c",
		`mv ${extractLocation}/x64/* ${wineDir}/drive_c/windows/system32/`,
		"&&",
		`mv ${extractLocation}/x86/* ${wineDir}/drive_c/windows/syswow64`,
	]).execute();

	// d3d12
	await wineCommand([
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12 /t REG_SZ /d native /f`,
	]);

	// d3d12core
	await wineCommand([
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12core /t REG_SZ /d native /f`,
	]);

	await remove(downloadLocation);
	await remove(extractLocation, {
		recursive: true,
	});
};

export const updateWinetricksModules = async () => {
	// Trying to install a few different redists to ensure that older games will still run properly.
	// Not sure if just vcrun2022/vcrun2026 will be able to do this
	await winetricksCommand(["vcrun2019 vcrun2022 vcrun2026 dxvk"]);
};

const wineCommand = async (commands: string[]) => {
	const appDir = await resourceDir();
	const winePrefix = await join(appDir, "wine");
	if (!(await exists(winePrefix))) {
		await createWineEnvironment();
	}

	await Command.create("sh", ["-c", `./wine/bin/wine ${commands.join(" ")}`], {
		env: {
			WINEPREFIX: winePrefix,
		},
	}).execute();
};

const winetricksCommand = async (commands: string[]) => {
	const appDir = await resourceDir();

	const winePrefix = await join(appDir, "wine");
	if (!(await exists(winePrefix))) {
		await updateWine();
		await updateWinetricks();
	}

	await Command.create(
		"sh",
		["-c", `./wine/winetricks -q ${commands.join(" ")}`],
		{
			env: {
				WINEPREFIX: winePrefix,
			},
		},
	).execute();
};
