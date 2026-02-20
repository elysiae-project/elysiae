import { downloadFile, getGithubInfo } from "./WebUtils";
import { exists, mkdir, remove } from "@tauri-apps/plugin-fs";
import { join, resourceDir } from "@tauri-apps/api/path";
import { extractFile, getAllDirs, moveDirItems } from "./FileUtils";
import { Command } from "@tauri-apps/plugin-shell";

export const createWineEnvironment = async () => {
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
	await setTimeout(() => {}, 10000);
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
	// d3d12
	await wineCommand([
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12 /t REG_SZ /d native /f`,
	]);

	// d3d12core
	await wineCommand([
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12core /t REG_SZ /d native /f`,
	]);

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

	await Command.create(
		"sh",
		["-c", `${winePrefix}/bin/wine ${commands.join(" ")}`],
		{
			env: {
				WINEPREFIX: winePrefix,
			},
		},
	).execute();
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
		["-c", `${winePrefix}/winetricks -q ${commands.join(" ")}`],
		{
			env: {
				WINEPREFIX: winePrefix,
			},
		},
	).execute();
};
